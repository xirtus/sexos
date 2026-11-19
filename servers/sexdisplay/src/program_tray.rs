// SPDX-License-Identifier: GPL-3.0-only
//! Program tray items, their menus and tooltips, and notification cards with
//! buttons and an inline reply field (slice S2). The Shell decides what shows
//! and where (`sex_pdx::program_tray::OP_TRAY_VIEW`, by sections, each shown
//! only once whole); Display draws it over the desktop with the layouts both
//! sides share, so the Shell's clicks land on what is drawn here, in the
//! style of the window menu. Only the Shell may send it.
use sex_pdx::program_tray::{CardLayout, MenuLayout, Receiver, Section};

const BG: u32 = 0x0018_1825;
const RIM: u32 = 0x0045_475A;
const PICKED: u32 = 0x0031_3244;
const INK: u32 = 0x00CD_D6F4;
const DIM: u32 = 0x006C_7086;
const FIELD: u32 = 0x0011_111B;
const BUTTON: u32 = 0x0031_3244;
const SEND: u32 = 0x0035_74E8;
const CLOSE_INK: u32 = 0x00F3_8BA8;
const SHELL_PD: u32 = 3;

static mut VIEW: Receiver = Receiver::new();
/// Bumped on each commit; a draw receipt is written once per change.
static mut CHANGES: u32 = 0;
static mut DRAWN: (u32, u32) = (0, 0);

/// One `OP_TRAY_VIEW` call. True when what shows changed (a section committed).
pub fn receive(caller: u32, a0: u64, a1: u64, a2: u64) -> bool {
    if caller != SHELL_PD { return false; }
    let r = unsafe { &mut *core::ptr::addr_of_mut!(VIEW) };
    let Some(section) = r.push(a0, a1, a2) else { return false };
    unsafe { CHANGES = CHANGES.wrapping_add(1); }
    static mut BUDGET: u32 = 256;
    unsafe {
        if BUDGET > 0 {
            BUDGET -= 1;
            let v = &r.shown;
            match section {
                Section::Icons => { sex_pdx::serial_record!("[display.tray_items] section=icons count={}", v.icon_count); }
                Section::Menu => { sex_pdx::serial_record!("[display.tray_items] section=menu shown={} rows={} hover={}", v.menu.is_some() as u8,
                    v.menu.map_or(0, |m| m.rows), v.menu.and_then(|m| m.hover).map_or(-1, |h| h as i32)); }
                Section::Tip => { sex_pdx::serial_record!("[display.tray_items] section=tip shown={}", v.tip.is_some() as u8); }
                Section::Card => { sex_pdx::serial_record!("[display.tray_items] section=card shown={} actions={} reply={}", v.card.is_some() as u8,
                    v.card.map_or(0, |c| c.actions), v.card.map_or(0, |c| c.reply as u8)); }
                Section::Reply => { sex_pdx::serial_record!("[display.tray_items] section=reply active={} len={}", v.reply.active as u8, v.reply.text.len); }
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

/// A panel with a one-pixel (scaled) rim.
fn panel(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), s: usize, bg: u32) {
    fill(fb, w, h, total, r, RIM);
    fill(fb, w, h, total, (r.0 + s, r.1 + s, r.2.saturating_sub(2 * s), r.3.saturating_sub(2 * s)), bg);
}

/// Text at (x0, y0) in `ink`, cut at `max_w`; returns the width drawn.
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

/// Text in the middle of `r` vertically, `inset` from its left.
fn text_in(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], r: (usize, usize, usize, usize), inset: usize, ink: u32, scale: u32) -> usize {
    let line = super::glyphs::UI_H * scale as usize;
    text(fb, w, h, total, t, r.0 + inset, r.1 + r.3.saturating_sub(line) / 2, ink, scale, r.2.saturating_sub(2 * inset))
}

/// Text centred in `r`.
fn text_centre(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], r: (usize, usize, usize, usize), ink: u32, scale: u32) {
    let tw = super::glyphs::ui_width(t, scale).unwrap_or(0).min(r.2);
    let line = super::glyphs::UI_H * scale as usize;
    text(fb, w, h, total, t, r.0 + (r.2 - tw) / 2, r.1 + r.3.saturating_sub(line) / 2, ink, scale, r.2);
}

/// Dark or light ink, whichever reads on `bg`.
fn ink_on(bg: u32) -> u32 {
    let (r, g, b) = ((bg >> 16) & 0xFF, (bg >> 8) & 0xFF, bg & 0xFF);
    if r * 299 + g * 587 + b * 114 > 140_000 { 0x0011_111B } else { 0x00F5_F5F5 }
}

/// Draws the items in the panel, then an open menu, a tooltip and the card
/// (after the body passes, before the quick menu and the window menu).
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32, bar: usize) {
    let v = unsafe { &(*core::ptr::addr_of!(VIEW)).shown };
    let s = scale.max(1) as usize;
    for ic in v.icons() {
        let size = ic.size as usize;
        if size == 0 { continue; }
        // A tile inset a little in its square, with the label on it.
        let inset = (size / 8).max(1);
        let tile = (ic.x as usize + inset, ic.y as usize + inset, size - 2 * inset, size - 2 * inset);
        fill(fb, w, h, total, tile, ic.colour);
        text_centre(fb, w, h, total, &ic.label[..ic.label_len as usize], tile, ink_on(ic.colour), scale);
    }
    if let Some(m) = v.menu {
        if let Some(l) = MenuLayout::new(m.x as usize, m.y as usize, m.rows as usize, w, h, s) {
            panel(fb, w, h, total, l.panel, s, BG);
            for i in 0..l.rows {
                let r = l.row(i);
                if m.hover == Some(i as u8) {
                    fill(fb, w, h, total, r, PICKED);
                    fill(fb, w, h, total, (r.0, r.1 + 3 * s, 3 * s, r.3.saturating_sub(6 * s)), sex_pdx::desktop_style::FOCUS);
                }
                text_in(fb, w, h, total, m.text[i].as_bytes(), r, 10 * s, INK, scale);
            }
            unsafe { if DRAWN.0 != CHANGES { DRAWN.0 = CHANGES; sex_pdx::serial_record!("[display.tray_items.draw] menu rows={} panel={}x{}+{}+{}", l.rows, l.panel.2, l.panel.3, l.panel.0, l.panel.1); } }
        }
    }
    if let Some(t) = v.tip.filter(|_| v.menu.is_none()) {
        let tw = super::glyphs::ui_width(t.text.as_bytes(), scale).unwrap_or(0) + 16 * s;
        let th = super::glyphs::UI_H * s + 8 * s;
        let x = (t.x as usize).min(w.saturating_sub(tw));
        let r = (x, t.y as usize, tw, th);
        panel(fb, w, h, total, r, s, BG);
        text_in(fb, w, h, total, t.text.as_bytes(), r, 8 * s, INK, scale);
    }
    let Some(c) = v.card else { return };
    let Some(l) = CardLayout::new(w, bar, s, c.actions as usize, c.reply) else { return };
    panel(fb, w, h, total, l.panel, s, BG);
    // "From: summary", then the body, cut to the card.
    let used = text_in(fb, w, h, total, c.from.as_bytes(), l.head, 0, DIM, scale);
    let gap = 8 * s;
    let head_rest = (l.head.0 + used + gap, l.head.1, l.head.2.saturating_sub(used + gap), l.head.3);
    text_in(fb, w, h, total, c.summary(), head_rest, 0, INK, scale);
    let body = c.body();
    let line = &body[..body.iter().position(|b| *b == b'\n').unwrap_or(body.len())];
    text_in(fb, w, h, total, line, l.body, 0, INK, scale);
    // Close: an x in its corner.
    let (cx, cy, cw, ch) = l.close;
    let n = cw.min(ch).saturating_sub(s);
    for i in (0..n).step_by(1) {
        fill(fb, w, h, total, (cx + i, cy + i, s, s), CLOSE_INK);
        fill(fb, w, h, total, (cx + n - 1 - i, cy + i, s, s), CLOSE_INK);
    }
    for i in 0..l.action_count {
        let r = l.actions[i];
        fill(fb, w, h, total, r, BUTTON);
        text_centre(fb, w, h, total, c.labels[i].as_bytes(), r, INK, scale);
    }
    if let (Some(f), Some(send)) = (l.field, l.send) {
        let active = v.reply.active;
        fill(fb, w, h, total, f, if active { sex_pdx::desktop_style::FOCUS } else { RIM });
        let inner = (f.0 + s, f.1 + s, f.2.saturating_sub(2 * s), f.3.saturating_sub(2 * s));
        fill(fb, w, h, total, inner, FIELD);
        let typed = v.reply.text.as_bytes();
        // The end of a long reply shows, as in any one-line field.
        let room = inner.2.saturating_sub(16 * s);
        let mut from = 0;
        while from < typed.len() && super::glyphs::ui_width(&typed[from..], scale).unwrap_or(0) > room { from += 1; }
        let tw = if typed.is_empty() && !active { text_in(fb, w, h, total, c.placeholder.as_bytes(), inner, 8 * s, DIM, scale); 0 }
            else { text_in(fb, w, h, total, &typed[from..], inner, 8 * s, INK, scale) };
        if active {
            let line = super::glyphs::UI_H * s;
            fill(fb, w, h, total, (inner.0 + 8 * s + tw + s, inner.1 + inner.3.saturating_sub(line) / 2, s.max(1) * 2, line), INK);
        }
        fill(fb, w, h, total, send, SEND);
        text_centre(fb, w, h, total, b"Send", send, 0x00F5_F5F5, scale);
    }
    unsafe {
        if DRAWN.1 != CHANGES {
            DRAWN.1 = CHANGES;
            sex_pdx::serial_record!("[display.tray_items.draw] card actions={} reply={} active={} len={} panel={}x{}+{}+{}", l.action_count, c.reply as u8,
                v.reply.active as u8, v.reply.text.len, l.panel.2, l.panel.3, l.panel.0, l.panel.1);
        }
    }
}
