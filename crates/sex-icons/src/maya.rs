// SPDX-License-Identifier: GPL-3.0-only
//! The twenty Maya day signs (the Dreamspell's solar seals), drawn from
//! `assets/maya/day-signs.txt`: vector traces of the classic day-sign drawings
//! (CJLL Wright, Wikimedia Commons, CC BY 2.5; `assets/maya/ATTRIBUTION.md`),
//! written by `scripts/brand/maya_glyphs.py`.
//!
//! Each sign has two filled layers on the 24-unit icon grid: `back`, the
//! cartouche's silhouette in the seal's colour, and `ink`, the drawing, whose
//! holes are wound against their outlines. Unlike the icon set's fills these
//! keep their winding (nonzero rule), so the holes stay open.

use super::{over, walk, Place, Sink};
use sex_font::raster::{Raster, FX};

pub static SOURCE: &str = include_str!("../../../assets/maya/day-signs.txt");

/// The `(back, ink)` path data of seal 1..=20 (1 = Imix, Red Dragon; 20 = Ajaw, Yellow Sun).
pub fn sign(seal: u8) -> Option<(&'static str, &'static str)> {
    let mut back = None;
    for l in SOURCE.lines() {
        if l.starts_with('#') { continue; }
        let mut it = l.splitn(4, ' ');
        let (Some(n), Some(_name), Some(layer), Some(data)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        if n.parse::<u8>().ok() != Some(seal) { continue; }
        match layer { "back" => back = Some(data), "ink" => return back.map(|b| (b, data)), _ => {} }
    }
    None
}

/// Lines straight onto the raster: no point buffer, any path length.
struct Filler<'r, 'a> { raster: &'r mut Raster<'a>, place: Place, start: (f32, f32), at: (f32, f32), open: bool }

impl Filler<'_, '_> {
    fn dev(&self, (x, y): (f32, f32)) -> (i32, i32) {
        let f = |v: f32| { let v = v * FX as f32; (if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i32 };
        (f(self.place.x + x * self.place.scale), f(self.place.y + y * self.place.scale))
    }
    fn to(&mut self, p: (f32, f32)) {
        let (a, b) = (self.dev(self.at), self.dev(p));
        self.raster.line(a.0, a.1, b.0, b.1);
        self.at = p;
    }
    fn end(&mut self) { if self.open { self.to(self.start); self.open = false; } }
    fn steps(&self, len: f32) -> usize { ((len * self.place.scale / 1.5) as usize).clamp(2, 32) }
}

impl Sink for Filler<'_, '_> {
    fn begin(&mut self, x: f32, y: f32, _fill: bool) { self.end(); self.start = (x, y); self.at = (x, y); self.open = true; }
    fn line_to(&mut self, x: f32, y: f32) { self.to((x, y)); }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.at;
        let n = self.steps(libm::fabsf(x1 - p0.0) + libm::fabsf(y1 - p0.1) + libm::fabsf(x - x1) + libm::fabsf(y - y1));
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            self.to((u * u * p0.0 + 2.0 * u * t * x1 + t * t * x, u * u * p0.1 + 2.0 * u * t * y1 + t * t * y));
        }
    }
    fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.at;
        let n = self.steps(libm::fabsf(x1 - p0.0) + libm::fabsf(y1 - p0.1) + libm::fabsf(x2 - x1) + libm::fabsf(y2 - y1)
            + libm::fabsf(x - x2) + libm::fabsf(y - y2));
        for i in 1..=n {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.to((a * p0.0 + b * x1 + c * x2 + d * x, a * p0.1 + b * y1 + c * y2 + d * y));
        }
    }
    fn close(&mut self) { self.end(); }
    fn circle(&mut self, _cx: f32, _cy: f32, _r: f32, _filled: bool) {}
}

/// Coverage of `data` filled by the nonzero rule (winding kept), placed by
/// `place`, into a `w` x `h` mask. `acc` needs `sex_font::raster::scratch_len(w, h)` cells.
pub fn coverage(data: &str, place: Place, w: usize, h: usize, acc: &mut [i32], out: &mut [u8]) -> bool {
    if out.len() < w * h { return false; }
    let Some(mut raster) = Raster::new(w, h, acc) else { return false };
    let mut f = Filler { raster: &mut raster, place, start: (0.0, 0.0), at: (0.0, 0.0), open: false };
    let ok = walk(data, &mut f);
    f.end();
    raster.finish(&mut out[..w * h]);
    ok
}

/// Draws seal 1..=20 as a `side` x `side` image of non-premultiplied `0xAARRGGBB`
/// pixels into `out` (cleared first): the cartouche in `colour`, the drawing in
/// `ink`. `false` for an unknown seal or buffers that are too small.
pub fn draw(seal: u8, side: usize, colour: u32, ink: u32, acc: &mut [i32], mask: &mut [u8], out: &mut [u32]) -> bool {
    let Some((back, lines)) = sign(seal) else { return false };
    if out.len() < side * side || mask.len() < side * side { return false; }
    out[..side * side].fill(0);
    let place = Place::square(side);
    for (data, c) in [(back, colour), (lines, ink)] {
        if !coverage(data, place, side, side, acc, mask) { return false; }
        for (o, &a) in out[..side * side].iter_mut().zip(&mask[..side * side]) { *o = over(*o, c, a as u32); }
    }
    true
}
