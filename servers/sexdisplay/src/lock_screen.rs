// SPDX-License-Identifier: GPL-3.0-only
//! The lock surface: while the Shell says the session is locked, it covers the whole
//! frame, so no window, title or notice shows. It says what to do, one dot per typed
//! character, and why the last try failed. The Shell keeps the keys; Display only
//! paints (`sex_pdx::screen_state`).
use sex_pdx::screen_state::{self as s, Lock};

// The SexOS wallpaper's charcoal, with the lips above the prompt (sex_pdx::brand).
const TOP: u32 = sex_pdx::brand::WALL_TOP;
const BOTTOM: u32 = sex_pdx::brand::WALL_BOTTOM;
const INK: u32 = 0x00E8_ECF4;
const SOFT: u32 = 0x00A0_A8BC;
const WARN: u32 = 0x00FF_9C8A;

fn line(fb: *mut u32, w: usize, h: usize, total: usize, text: &[u8], y0: usize, scale: u32, ink: u32) {
    let Some(width) = super::glyphs::ui_width(text, scale) else { return };
    let x0 = w.saturating_sub(width) / 2;
    let lh = super::glyphs::UI_H * scale as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width + 1).min(w) {
            let at = y * w + x;
            if at >= total { return; }
            let Some(alpha) = super::glyphs::ui_coverage(text, (x - x0) as i32, y - y0, scale) else { return };
            if alpha == 0 { continue; }
            unsafe {
                let p = fb.add(at);
                core::ptr::write_volatile(p, super::coverage_blend(ink, core::ptr::read_volatile(p), alpha));
            }
        }
    }
}

/// Paints the lock surface over the whole target when locked; false otherwise.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize) -> bool {
    let lock: Lock = s::lock();
    if !lock.locked || w == 0 || h == 0 { return false; }
    for y in 0..h {
        let color = super::vertical_gradient_xrgb(y, h, TOP, BOTTOM);
        for x in 0..w {
            let at = y * w + x;
            if at >= total { break; }
            unsafe { core::ptr::write_volatile(fb.add(at), color) };
        }
    }
    let scale = sex_pdx::desktop_style::ui_scale().clamp(1, 3);
    let lh = super::glyphs::UI_H * scale as usize;
    let mid = h / 2;
    let title_y = mid.saturating_sub(3 * lh);
    let mark_w = (w / 7).min(360) as u32;
    let mark_h = sex_pdx::brand::height_for(mark_w) as usize;
    if let Some(mark_y) = title_y.checked_sub(mark_h + 2 * lh) {
        let mark_x = (w - mark_w as usize) / 2;
        let under = |_: u32, y: u32| super::vertical_gradient_xrgb(mark_y + y as usize, h, TOP, BOTTOM);
        sex_pdx::brand::draw(mark_w, sex_pdx::brand::Tone::Rich, under, |x, y, c| {
            let at = (mark_y + y as usize) * w + mark_x + x as usize;
            if at < total { unsafe { core::ptr::write_volatile(fb.add(at), c) }; }
        });
    }
    line(fb, w, h, total, b"Locked", title_y, (scale * 2).min(3), INK);
    let mut dots = [b' '; 2 * 64];
    let n = (lock.typed as usize).min(64);
    for i in 0..n { dots[2 * i] = b'*'; }
    let dots = if n == 0 { &b"_"[..] } else { &dots[..2 * n - 1] };
    line(fb, w, h, total, dots, mid, scale, INK);
    let say: &[u8] = match lock.say {
        s::SAY_WRONG => b"That password is not right. Try again.",
        s::SAY_CHECKING => b"Checking...",
        s::SAY_NO_PASSWORD => b"No password is set. Press Enter to unlock; set one in Settings.",
        _ => b"Type your password and press Enter.",
    };
    line(fb, w, h, total, say, mid + 2 * lh, scale, if lock.say == s::SAY_WRONG { WARN } else { SOFT });
    true
}
