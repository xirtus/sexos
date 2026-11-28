// SPDX-License-Identifier: GPL-3.0-only
//! The calendar drop-down's oracle band (`sex_pdx::tray::Calendar::oracle`),
//! laid out like the 13 Moon day charts (`sex_pdx::tray::Oracle`): the day's
//! Dreamspell kin (`sex_time::dreamspell`) as an oracle cross on the left —
//! guide above, antipode left, the day in the middle, analog right, occult
//! below — and its wave spell's 13 days as a U on the right, gates (days 1, 5,
//! 9 and 13) marked and today framed; under both, the day's name, its wave
//! spell and its castle.
//!
//! Every seal is its Maya day sign (`sex_icons::maya`, traced from the classic
//! drawings), the cartouche in the seal's colour, with its tone in Maya dots
//! and bars above. The menu redraws every frame while it slides, so each sign
//! is rasterized once per size and kept, as coverage (colour is added when
//! it is drawn).

use sex_pdx::tray::Oracle;
use sex_time::dreamspell::{Colour, Kin};

/// Largest sign: a cross cell's glyph at scale 3.
const CAP: usize = 88;
/// Enough for the cross and the wave spell at once (18 signs).
const SLOTS: usize = 18;
const INK: u32 = 0x1C1A1B;
/// Today's frame, the charts' green.
const TODAY: u32 = 0x3FB950;

struct Slot { seal: u8, side: usize, ok: bool, back: [u8; CAP * CAP], ink: [u8; CAP * CAP] }
struct Cache { slots: [Slot; SLOTS], next: usize, acc: [i32; sex_font::raster::scratch_len(CAP, CAP)] }
const EMPTY: Slot = Slot { seal: 0, side: 0, ok: false, back: [0; CAP * CAP], ink: [0; CAP * CAP] };
static mut CACHE: Cache = Cache { slots: [EMPTY; SLOTS], next: 0, acc: [0; sex_font::raster::scratch_len(CAP, CAP)] };

fn seal_colour(c: Colour) -> u32 {
    match c { Colour::Red => 0xD8453E, Colour::White => 0xF2EFE9, Colour::Blue => 0x3F6FD1, Colour::Yellow => 0xF2C230 }
}

/// The day sign of `seal` at `side` pixels, (silhouette, ink) coverage, drawn once and kept.
#[allow(static_mut_refs)]
fn sign(seal: u8, side: usize) -> Option<(&'static [u8], &'static [u8])> {
    let side = side.min(CAP);
    let c = unsafe { &mut CACHE };
    let i = match c.slots.iter().position(|s| s.seal == seal && s.side == side) {
        Some(i) => i,
        None => {
            let i = c.next;
            c.next = (c.next + 1) % SLOTS;
            let slot = &mut c.slots[i];
            slot.ok = match sex_icons::maya::sign(seal) {
                Some((back, ink)) => {
                    let place = sex_icons::Place::square(side);
                    sex_icons::maya::coverage(back, place, side, side, &mut c.acc, &mut slot.back[..side * side])
                        && sex_icons::maya::coverage(ink, place, side, side, &mut c.acc, &mut slot.ink[..side * side])
                }
                None => false,
            };
            slot.seal = seal;
            slot.side = side;
            i
        }
    };
    let slot = &c.slots[i];
    slot.ok.then(|| (&slot.back[..side * side], &slot.ink[..side * side]))
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

/// A rounded rectangle of `c` at `a`.
fn rounded(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), k: usize, c: u32, a: u8) {
    for yy in 0..r.3 {
        for xx in 0..r.2 {
            let corner = |v: usize, n: usize| if v < k { k - v } else if v + k >= n { v + 1 + k - n } else { 0 };
            let (dx, dy) = (corner(xx, r.2), corner(yy, r.3));
            if dx * dx + dy * dy > k * k { continue; }
            put(fb, w, h, total, r.0 + xx, r.1 + yy, c, a);
        }
    }
}

/// One kin in `r`: with `tone`, its tone in Maya numerals (dots over bars) on
/// top and its day sign under them, else the sign alone (a wave spell day,
/// whose place gives its tone); `frame` draws a rounded backing first (today, a gate).
#[allow(clippy::too_many_arguments)]
fn cell(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), s: usize, kin: Kin, tone: bool, ink: u32, frame: Option<(u32, u8)>) {
    let (d, gap) = (2 * s, s);
    let strip = if tone { 3 * d + 2 * gap + s } else { s };
    let side = r.3.saturating_sub(strip + s).min(r.2.saturating_sub(2 * s)).min(CAP);
    if let Some((c, a)) = frame { rounded(fb, w, h, total, r, 4 * s, c, a); }
    let gx = r.0 + (r.2 - side) / 2;
    let gy = r.1 + strip;
    // Bars from the sign upwards, the dots on top.
    let (bars, dots) = if tone { ((kin.tone() / 5) as usize, (kin.tone() % 5) as usize) } else { (0, 0) };
    let mut row = gy - s;
    let bar_w = side * 2 / 3;
    for _ in 0..bars {
        row -= d;
        super::tray_fill(fb, w, h, total, (r.0 + (r.2 - bar_w) / 2, row, bar_w, d), ink);
        row -= gap;
    }
    if dots > 0 {
        row -= d;
        let span = dots * d + (dots - 1) * gap * 2;
        let left = r.0 + r.2.saturating_sub(span) / 2;
        for i in 0..dots { super::tray_fill(fb, w, h, total, (left + i * (d + 2 * gap), row, d, d), ink); }
    }
    if let Some((back, lines)) = sign(kin.seal(), side) {
        let colour = seal_colour(kin.colour());
        for yy in 0..side {
            for xx in 0..side {
                let i = yy * side + xx;
                put(fb, w, h, total, gx + xx, gy + yy, colour, back[i]);
                put(fb, w, h, total, gx + xx, gy + yy, INK, lines[i]);
            }
        }
    }
}

/// Draws the oracle band for `kin` into `band` on a panel of colour `bg`.
#[allow(clippy::too_many_arguments)]
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, band: (usize, usize, usize, usize), s: usize, scale: u32, kin: Kin, bg: u32) {
    let ink = sex_pdx::desktop_style::contrast_ink(bg);
    let o = Oracle::new(band, s);
    let or = kin.oracle();
    let today = Some((TODAY, 255));
    for (i, k) in [or.guide, or.antipode, or.destiny, or.analog, or.occult].into_iter().enumerate() {
        cell(fb, w, h, total, o.cross[i], s, k, true, ink, (i == Oracle::DESTINY).then_some((TODAY, 255)));
    }
    let first = kin.wave_spell().number();
    for (i, r) in o.wave.iter().enumerate() {
        let Some(k) = Kin::new(first + i as u16) else { continue };
        let frame = if k == kin { today } else if matches!(i + 1, 1 | 5 | 9 | 13) { Some((ink, 40)) } else { None };
        cell(fb, w, h, total, *r, s, k, false, ink, frame);
    }

    let line = super::glyphs::UI_H * s;
    let mut buf = [0u8; 48];
    let centred = |text: &[u8], r: (usize, usize, usize, usize), y: usize| {
        let tw = super::glyphs::ui_width(text, scale).unwrap_or(0).min(r.2);
        super::tray_text(fb, w, h, total, text, r.0 + (r.2 - tw) / 2, y, bg, scale, r.2);
    };
    // Inside the U: which wave spell, and the day's kin.
    let n = fmt(&mut buf, &[b"Wave spell "], Some(kin.wave_number() as u32), &[]);
    centred(&buf[..n], o.wave_label, o.wave_label.1 + o.wave_label.3.saturating_sub(line) / 2);
    let k = o.kin_label;
    let top = k.1 + k.3.saturating_sub(2 * line + s) / 2;
    let n = fmt(&mut buf, &[b"Kin "], Some(kin.number() as u32), &[]);
    centred(&buf[..n], k, top);
    let n = fmt(&mut buf, &[b"Day "], Some(kin.wave_day() as u32), &[b" of 13"]);
    centred(&buf[..n], k, top + line + s);

    // Under both: "White Electric Wizard", its wave spell, its castle.
    let (colour, animal) = kin.seal_parts();
    let step = sex_pdx::tray::ORACLE_LINE * s;
    let r = o.words;
    let n = fmt(&mut buf, &[colour.as_bytes(), b" ", kin.tone_name().as_bytes(), b" ", animal.as_bytes()], None, &[]);
    centred(&buf[..n], r, r.1);
    let n = fmt(&mut buf, &[kin.wave_spell().seal_name().as_bytes(), b" wave spell"], None, &[]);
    centred(&buf[..n], r, r.1 + step);
    centred(kin.castle().as_bytes(), r, r.1 + 2 * step);
}

fn fmt(out: &mut [u8; 48], before: &[&[u8]], num: Option<u32>, after: &[&[u8]]) -> usize {
    let mut n = 0;
    let mut push = |b: &[u8], n: &mut usize| for &c in b { if *n < out.len() { out[*n] = c; *n += 1; } };
    for b in before { push(b, &mut n); }
    if let Some(mut v) = num {
        let mut d = [0u8; 10];
        let mut k = 0;
        loop { d[k] = b'0' + (v % 10) as u8; k += 1; v /= 10; if v == 0 { break; } }
        while k > 0 { k -= 1; push(&[d[k]], &mut n); }
    }
    for b in after { push(b, &mut n); }
    n
}
