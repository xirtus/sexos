// SPDX-License-Identifier: GPL-3.0-only
//! Sixel graphics: `DCS P1 ; P2 ; P3 q <data> ST`, decoded as `vte` hands over the
//! bytes, then shown at the cursor as an image placement. Raster attributes (`"`),
//! colour definitions in HLS and RGB (`#`), repeats (`!`), carriage return (`$`) and
//! next band (`-`) are understood; `P2 = 1` leaves unset pixels transparent. Pixels are
//! square (the aspect ratio in P1 and the raster attributes is ignored, as xterm and
//! most modern terminals do). After the image the cursor goes to the line below it,
//! in its starting column (xterm's sixel scrolling mode, the default).

use crate::images::{CursorMove, Image, PlaceSpec};
use crate::Screen;
use alloc::vec;
use alloc::vec::Vec;

/// Colour registers a sixel image may use.
pub const REGISTERS: usize = 256;
/// Largest side of a sixel image, in pixels.
pub const MAX_SIDE: usize = 4096;

/// The VT340's default palette, in percent RGB.
const VT340: [(u8, u8, u8); 16] = [
    (0, 0, 0), (20, 20, 80), (80, 13, 13), (20, 80, 20), (80, 20, 80), (20, 80, 80), (80, 80, 20), (53, 53, 53),
    (26, 26, 26), (33, 33, 60), (60, 26, 26), (33, 60, 33), (60, 33, 60), (33, 60, 60), (60, 60, 33), (80, 80, 80),
];

fn pct(v: u32) -> u32 { (v.min(100) * 255 + 50) / 100 }
fn rgb_pct(r: u32, g: u32, b: u32) -> u32 { pct(r) << 16 | pct(g) << 8 | pct(b) }

/// DEC HLS (hue 0 = blue, 120 = red, 240 = green; lightness and saturation in percent).
fn hls(h: u32, l: u32, s: u32) -> u32 {
    let h = ((h % 360) + 240) % 360; // to the usual hue circle, red at 0
    let (l, s) = (l.min(100) as f32 / 100.0, s.min(100) as f32 / 100.0);
    let c = (1.0 - (2.0 * l - 1.0).abs_f()) * s;
    // Integer sector arithmetic: no float remainder, which would need libm here.
    let hp = (h / 60 % 2) as f32 + (h % 60) as f32 / 60.0;
    let x = c * (1.0 - (hp - 1.0).abs_f());
    let (r, g, b) = match h / 60 { 0 => (c, x, 0.0), 1 => (x, c, 0.0), 2 => (0.0, c, x), 3 => (0.0, x, c), 4 => (x, 0.0, c), _ => (c, 0.0, x) };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m) * 255.0 + 0.5).clamp(0.0, 255.0) as u32;
    to(r) << 16 | to(g) << 8 | to(b)
}
trait AbsF { fn abs_f(self) -> f32; }
impl AbsF for f32 { fn abs_f(self) -> f32 { if self < 0.0 { -self } else { self } } }

/// One sixel image being decoded.
pub struct Decoder {
    palette: Vec<u32>,
    color: usize,
    transparent: bool,
    x: usize,
    band: usize,
    /// Pixels, 0xAARRGGBB with alpha 0 where nothing was drawn, `bw` per row.
    pixels: Vec<u32>,
    bw: usize,
    bh: usize,
    /// Extent drawn so far, and the raster attributes' size.
    w: usize,
    h: usize,
    raster: (usize, usize),
    /// A command collecting numbers: `!`, `"` or `#`, and its parameters.
    cmd: u8,
    params: [u32; 5],
    np: usize,
    repeat: usize,
}

impl Decoder {
    pub fn new(params: &[u16]) -> Self {
        let mut palette = vec![0u32; REGISTERS];
        for (i, &(r, g, b)) in VT340.iter().enumerate() { palette[i] = rgb_pct(r as u32, g as u32, b as u32); }
        Decoder {
            palette, color: 0, transparent: params.get(1) == Some(&1), x: 0, band: 0, pixels: Vec::new(), bw: 0, bh: 0,
            w: 0, h: 0, raster: (0, 0), cmd: 0, params: [0; 5], np: 0, repeat: 1,
        }
    }

    pub fn put(&mut self, byte: u8) {
        if self.cmd != 0 {
            match byte {
                b'0'..=b'9' => { if self.np == 0 { self.np = 1; } let p = &mut self.params[self.np - 1]; *p = p.saturating_mul(10).saturating_add((byte - b'0') as u32).min(1 << 20); return; }
                b';' => { if self.np == 0 { self.np = 1; } if self.np < 5 { self.np += 1; } return; }
                _ => self.finish_command(),
            }
        }
        match byte {
            b'!' | b'"' | b'#' => { self.cmd = byte; self.params = [0; 5]; self.np = 0; }
            b'$' => self.x = 0,
            b'-' => { self.x = 0; self.band += 1; }
            b'?'..=b'~' => { let n = core::mem::replace(&mut self.repeat, 1); self.sixel(byte - b'?', n); }
            _ => {}
        }
    }

    fn finish_command(&mut self) {
        let p = self.params;
        match self.cmd {
            b'!' => self.repeat = (p[0] as usize).clamp(1, MAX_SIDE),
            b'"' => { if self.np >= 4 { self.raster = ((p[2] as usize).min(MAX_SIDE), (p[3] as usize).min(MAX_SIDE)); } }
            b'#' => {
                let reg = p[0] as usize % REGISTERS;
                if self.np >= 5 {
                    self.palette[reg] = match p[1] { 1 => hls(p[2], p[3], p[4]), 2 => rgb_pct(p[2], p[3], p[4]), _ => self.palette[reg] };
                }
                self.color = reg;
            }
            _ => {}
        }
        self.cmd = 0;
    }

    fn grow(&mut self, w: usize, h: usize) -> bool {
        if w > MAX_SIDE || h > MAX_SIDE { return false; }
        if w <= self.bw && h <= self.bh { return true; }
        let (nw, nh) = (w.max(self.bw).next_power_of_two().clamp(64, MAX_SIDE), h.max(self.bh).next_power_of_two().clamp(64, MAX_SIDE));
        let (nw, nh) = (nw.max(w), nh.max(h));
        let mut next: Vec<u32> = Vec::new();
        if next.try_reserve_exact(nw * nh).is_err() { return false; }
        next.resize(nw * nh, 0);
        for y in 0..self.bh { next[y * nw..y * nw + self.bw].copy_from_slice(&self.pixels[y * self.bw..(y + 1) * self.bw]); }
        self.pixels = next;
        self.bw = nw;
        self.bh = nh;
        true
    }

    fn sixel(&mut self, bits: u8, n: usize) {
        let (x0, y0) = (self.x, self.band * 6);
        let n = n.min(MAX_SIDE.saturating_sub(x0));
        self.x += n;
        if n == 0 || bits == 0 { self.w = self.w.max(self.x.min(MAX_SIDE)); return; }
        let top = 8 - bits.leading_zeros() as usize; // rows used in this band
        if !self.grow(x0 + n, y0 + top) { return; }
        let color = 0xFF00_0000 | self.palette[self.color];
        for bit in 0..6 {
            if bits & (1 << bit) == 0 { continue; }
            let row = (y0 + bit) * self.bw;
            self.pixels[row + x0..row + x0 + n].fill(color);
        }
        self.w = self.w.max(x0 + n);
        self.h = self.h.max(y0 + top);
    }

    /// The finished image, `None` when it has no pixels.
    pub fn finish(mut self) -> Option<Image> {
        if self.cmd != 0 { self.finish_command(); }
        let (w, h) = (self.w.max(self.raster.0), self.h.max(self.raster.1));
        if w == 0 || h == 0 || !self.grow(w, h) { return None; }
        let bg = if self.transparent { 0 } else { 0xFF00_0000 | self.palette[0] };
        let mut pixels = Vec::new();
        if pixels.try_reserve_exact(w * h).is_err() { return None; }
        for y in 0..h {
            pixels.extend(self.pixels[y * self.bw..y * self.bw + w].iter().map(|&p| if p >> 24 == 0 { bg } else { p }));
        }
        Some(Image { id: 0, number: 0, width: w, height: h, pixels })
    }
}

impl Screen {
    /// `DCS ... q`: a sixel image starts.
    pub(crate) fn sixel_start(&mut self, params: &vte::Params) {
        let p: Vec<u16> = params.iter().map(|p| p[0]).collect();
        self.images.sixel = Some(Decoder::new(&p));
    }
    pub(crate) fn sixel_put(&mut self, byte: u8) { if let Some(d) = self.images.sixel.as_mut() { d.put(byte); } }
    /// The image is complete: show it at the cursor.
    pub(crate) fn sixel_end(&mut self) {
        let Some(image) = self.images.sixel.take().and_then(Decoder::finish) else { return };
        let Some(key) = self.add_image(image) else { return };
        self.place_at_cursor(key, PlaceSpec::default(), CursorMove::Sixel);
    }
}
