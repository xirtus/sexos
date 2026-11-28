// SPDX-License-Identifier: GPL-3.0-only
//! The calendar drop-down's oracle band (`sex_pdx::tray::Calendar::oracle`):
//! the day's Dreamspell kin (`sex_time::dreamspell`) as an oracle cross —
//! guide above, antipode left, the day's seal in the middle, analog right,
//! occult below — each seal one of our own `seal-*` line icons on a tile of
//! its colour with its tone in Maya dots and bars above, and beside the cross
//! the kin, its tone and seal, and its wave spell.
//!
//! The menu redraws every frame while it slides, so each seal is rasterized
//! once per size and kept.

use sex_time::dreamspell::{Colour, Kin};

/// Largest tile glyph: 24 logical pixels at scale 3.
const CAP: usize = 72;
const SLOTS: usize = 5;
const INK: u32 = 0x1C1A1B;

struct Slot { seal: u8, side: usize, ok: bool, pixels: [u32; CAP * CAP] }
struct Cache { slots: [Slot; SLOTS], next: usize, acc: [i32; sex_font::raster::scratch_len(CAP, CAP)], mask: [u8; CAP * CAP] }
const EMPTY: Slot = Slot { seal: 0, side: 0, ok: false, pixels: [0; CAP * CAP] };
static mut CACHE: Cache = Cache { slots: [EMPTY; SLOTS], next: 0, acc: [0; sex_font::raster::scratch_len(CAP, CAP)], mask: [0; CAP * CAP] };

fn tile_colour(c: Colour) -> u32 {
    match c { Colour::Red => 0xD8453E, Colour::White => 0xF2EFE9, Colour::Blue => 0x3F6FD1, Colour::Yellow => 0xF2C230 }
}

/// The seal glyph of `kin` at `side` pixels, drawn once and kept.
#[allow(static_mut_refs)]
fn glyph(kin: Kin, side: usize) -> Option<&'static [u32]> {
    let side = side.min(CAP);
    let c = unsafe { &mut CACHE };
    let seal = kin.seal();
    let i = match c.slots.iter().position(|s| s.seal == seal && s.side == side) {
        Some(i) => i,
        None => {
            let i = c.next; c.next = (c.next + 1) % SLOTS;
            let palette = sex_icons::Palette { ink: INK, ..sex_icons::Palette::BRAND };
            let slot = &mut c.slots[i];
            slot.ok = sex_icons::draw(sex_icons::Kind::Line, kin.icon(), side, &palette, &mut c.acc, &mut c.mask[..side * side], &mut slot.pixels[..side * side]);
            slot.seal = seal; slot.side = side;
            i
        }
    };
    let slot = &c.slots[i];
    slot.ok.then(|| &slot.pixels[..side * side])
}

fn put(fb: *mut u32, w: usize, h: usize, total: usize, x: usize, y: usize, c: u32, a: u8) {
    let i = y * w + x;
    if x >= w || y >= h || i >= total || a == 0 { return; }
    unsafe {
        let p = fb.add(i);
        let v = if a == 255 { c } else { super::coverage_blend(c, core::ptr::read_volatile(p), a) };
        core::ptr::write_volatile(p, v);
    }
}

/// One cross cell at (x, y), `cell` pixels square: tone marks, then the tile.
fn cell(fb: *mut u32, w: usize, h: usize, total: usize, x: usize, y: usize, cell: usize, s: usize, kin: Kin, ink: u32, ring: Option<u32>) {
    let strip = 9 * s;
    let tile = cell - strip - 3 * s;
    let tx = x + (cell - tile) / 2;
    let ty = y + strip + s;
    // Tone in Maya numerals: dots for ones over bars for fives.
    let (bars, dots) = ((kin.tone() / 5) as usize, (kin.tone() % 5) as usize);
    let mut row = y + strip;
    for _ in 0..bars {
        row -= 3 * s;
        super::tray_fill(fb, w, h, total, (tx + 2 * s, row, tile - 4 * s, 2 * s), ink);
    }
    if dots > 0 {
        row -= 3 * s;
        let span = dots * 3 * s + (dots - 1) * 2 * s;
        let left = tx + tile.saturating_sub(span) / 2;
        for d in 0..dots { super::tray_fill(fb, w, h, total, (left + d * 5 * s, row, 3 * s, 3 * s), ink); }
    }
    if let Some(r) = ring { super::tray_fill(fb, w, h, total, (tx - 2 * s, ty - 2 * s, tile + 4 * s, tile + 4 * s), r); }
    // Tile with softened corners.
    let bg = tile_colour(kin.colour());
    let k = 3 * s;
    for yy in 0..tile {
        for xx in 0..tile {
            let corner = |a: usize| if a < k { k - a } else if a >= tile - k { a + 1 + k - tile } else { 0 };
            let (dx, dy) = (corner(xx), corner(yy));
            if dx * dx + dy * dy > k * k { continue; }
            put(fb, w, h, total, tx + xx, ty + yy, bg, 255);
        }
    }
    let side = (tile - 4 * s).min(CAP);
    if let Some(px) = glyph(kin, side) {
        let (gx, gy) = (tx + (tile - side) / 2, ty + (tile - side) / 2);
        for yy in 0..side { for xx in 0..side {
            let p = px[yy * side + xx];
            put(fb, w, h, total, gx + xx, gy + yy, p & 0xFF_FFFF, (p >> 24) as u8);
        } }
    }
}

/// Draws the oracle band for `kin` into `band` on a panel of colour `bg`.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, band: (usize, usize, usize, usize), s: usize, scale: u32, kin: Kin, bg: u32, ring: u32) {
    let ink = sex_pdx::desktop_style::contrast_ink(bg);
    let c = 36 * s;
    let (x, y) = (band.0, band.1 + band.3.saturating_sub(3 * c) / 2);
    let o = kin.oracle();
    cell(fb, w, h, total, x + c, y, c, s, o.guide, ink, None);
    cell(fb, w, h, total, x, y + c, c, s, o.antipode, ink, None);
    cell(fb, w, h, total, x + c, y + c, c, s, o.destiny, ink, Some(ring));
    cell(fb, w, h, total, x + 2 * c, y + c, c, s, o.analog, ink, None);
    cell(fb, w, h, total, x + c, y + 2 * c, c, s, o.occult, ink, None);

    // Words beside the cross.
    let tx = x + 3 * c + 8 * s;
    let tw = (band.0 + band.2).saturating_sub(tx);
    let step = super::glyphs::UI_H * s + s;
    let mut buf = [0u8; 40];
    let mut ty = band.1;
    let mut line = |text: &[u8], ty: &mut usize| { super::tray_text(fb, w, h, total, text, tx, *ty, bg, scale, tw); *ty += step; };
    let n = fmt(&mut buf, &[b"Kin "], Some(kin.number() as u32), &[]);
    line(&buf[..n], &mut ty);
    line(kin.tone_name().as_bytes(), &mut ty);
    line(kin.seal_name().as_bytes(), &mut ty);
    ty += s * 3;
    let n = fmt(&mut buf, &[b"Wave spell "], Some(kin.wave_number() as u32), &[]);
    line(&buf[..n], &mut ty);
    line(kin.wave_spell().seal_name().as_bytes(), &mut ty);
    let n = fmt(&mut buf, &[b"Day "], Some(kin.wave_day() as u32), &[b" of 13"]);
    line(&buf[..n], &mut ty);
}

fn fmt(out: &mut [u8; 40], before: &[&[u8]], num: Option<u32>, after: &[&[u8]]) -> usize {
    let mut n = 0;
    let mut push = |b: &[u8], n: &mut usize| for &c in b { if *n < out.len() { out[*n] = c; *n += 1; } };
    for b in before { push(b, &mut n); }
    if let Some(mut v) = num {
        let mut d = [0u8; 10]; let mut k = 0;
        loop { d[k] = b'0' + (v % 10) as u8; k += 1; v /= 10; if v == 0 { break; } }
        while k > 0 { k -= 1; push(&[d[k]], &mut n); }
    }
    for b in after { push(b, &mut n); }
    n
}
