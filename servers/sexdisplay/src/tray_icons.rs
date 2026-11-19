// SPDX-License-Identifier: GPL-3.0-only
//! SilkBar's tray icons (network, volume, battery) from the SexOS icon set
//! (`sex_icons`, assets/icons/icons.txt): the same rounded line style as the
//! launcher, Files and every Weft program, anti-aliased at any bar scale.
//!
//! Display has no heap: each chip's icon is drawn once into a fixed mask when its
//! size or state changes, and pixels are looked up from there.
use sex_pdx::tray::Status;

/// Largest icon side (16 logical pixels at scale 3).
const CAP: usize = 48;
const SLOTS: usize = 3;

/// Ink of the icons: the dark theme's text, its disabled text, and warning/charging.
pub const FG: u32 = 0x00F1EEF4;
pub const DIM: u32 = 0x006E6875;
pub const WARN: u32 = 0x00F38BA8;
pub const GOOD: u32 = 0x00A6E3A1;

/// What one chip shows: (icon, battery fill in percent or 255 for none, colour, side).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look { icon: &'static str, fill: u8, color: u32, side: usize }

struct Cache {
    looks: [Option<Look>; SLOTS],
    masks: [[u8; CAP * CAP]; SLOTS],
    acc: [i32; sex_font::raster::scratch_len(CAP, CAP)],
}

static mut CACHE: Cache = Cache { looks: [None; SLOTS], masks: [[0; CAP * CAP]; SLOTS], acc: [0; sex_font::raster::scratch_len(CAP, CAP)] };

fn look(slot: usize, s: &Status, side: usize) -> Look {
    let side = side.min(CAP);
    match slot {
        0 => {
            let on = matches!(s.wifi, 0 | 10);
            Look { icon: if on { "wifi" } else { "wifi-off" }, fill: 255, color: if on { FG } else { DIM }, side }
        }
        1 => {
            let icon = if s.muted { "volume-off" } else if s.volume == 0 { "speaker" } else if s.volume <= 50 { "volume-low" } else { "volume" };
            Look { icon, fill: 255, color: if s.muted { DIM } else { FG }, side }
        }
        _ => match s.battery {
            Some(p) => Look { icon: "battery", fill: p.min(100), color: if s.charging { GOOD } else if p <= 20 { WARN } else { FG }, side },
            None => Look { icon: "plug", fill: 255, color: FG, side },
        },
    }
}

/// The battery's outline without its built-in fill (the part after `F`).
fn outline(data: &str) -> &str { data.split('F').next().unwrap_or(data) }

/// Writes "F M5.5 9.5 H<x> V14.5 H5.5 Z" for a fill to `pct` into `buf`.
fn fill_path(pct: u8, buf: &mut [u8; 48]) -> &str {
    // The inner box runs from 5.5 to 16 grid units; at least a sliver shows.
    let tenths = 55 + (105 * pct.min(100) as usize / 100).max(8);
    let mut n = 0;
    let mut put = |b: &[u8]| { buf[n..n + b.len()].copy_from_slice(b); n += b.len(); };
    put(b"F M5.5 9.5 H");
    let digits = [b'0' + (tenths / 100) as u8, b'0' + (tenths / 10 % 10) as u8, b'.', b'0' + (tenths % 10) as u8];
    if tenths >= 100 { put(&digits) } else { put(&digits[1..]) }
    put(b" V14.5 H5.5 Z");
    core::str::from_utf8(&buf[..n]).unwrap_or("")
}

/// (colour, coverage) of chip `slot`'s icon at (gx, gy) in a `side` pixel square.
#[allow(static_mut_refs)]
pub fn at(slot: usize, gx: usize, gy: usize, side: usize, s: &Status) -> Option<(u32, u8)> {
    if slot >= SLOTS || gx >= side.min(CAP) || gy >= side.min(CAP) { return None; }
    let want = look(slot, s, side);
    let c = unsafe { &mut CACHE };
    if c.looks[slot] != Some(want) {
        let data = sex_icons::find(sex_icons::Kind::Line, want.icon)?;
        let side = want.side;
        let place = sex_icons::Place::square(side);
        // Strokes never thinner than a device pixel and a bit, as everywhere in the set.
        let width = (2.0 * place.scale).max(1.3) / place.scale;
        let mask = &mut c.masks[slot][..side * side];
        sex_icons::coverage(if want.fill == 255 { data } else { outline(data) }, place, width, side, side, &mut c.acc, mask);
        if want.fill != 255 {
            let mut buf = [0u8; 48];
            let mut extra = [0u8; CAP * CAP];
            sex_icons::coverage(fill_path(want.fill, &mut buf), place, 0.0, side, side, &mut c.acc, &mut extra[..side * side]);
            for (m, e) in mask.iter_mut().zip(&extra[..side * side]) { *m = (*m).max(*e); }
        }
        c.looks[slot] = Some(want);
    }
    let cover = c.masks[slot][gy * want.side + gx];
    (cover != 0).then_some((want.color, cover))
}
