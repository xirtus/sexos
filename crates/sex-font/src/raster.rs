// SPDX-License-Identifier: GPL-3.0-only
//! Anti-aliased scan conversion of glyph outlines in integer arithmetic only.
//!
//! Each edge adds the signed area it covers to an accumulation buffer, one cell per
//! pixel; a running sum along each row then gives every pixel's exact coverage
//! (the signed-area accumulation method of font-rs and FreeType's "smooth"
//! rasterizer, written here in fixed point). Coordinates are in 1/256 pixel.
//! There is no allocation: the caller lends the buffers, so the same code runs in
//! a server with no heap and in a host test, with bit-identical results.

/// Fixed-point units per pixel for outline coordinates.
pub const FX: i32 = 256;
/// Accumulated value of a fully covered pixel.
const ONE: i32 = 1 << 16;

/// Accumulation cells needed for a `w` x `h` bitmap.
pub const fn scratch_len(w: usize, h: usize) -> usize { (w + 2) * h }

pub struct Raster<'a> {
    w: usize,
    h: usize,
    acc: &'a mut [i32],
}

impl<'a> Raster<'a> {
    /// A cleared raster over `acc`, which must hold `scratch_len(w, h)` cells.
    pub fn new(w: usize, h: usize, acc: &'a mut [i32]) -> Option<Self> {
        let acc = acc.get_mut(..scratch_len(w, h))?;
        acc.fill(0);
        Some(Self { w, h, acc })
    }

    /// One straight edge, in 1/256 pixel, y downwards.
    pub fn line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        if y0 == y1 { return; }
        let (dir, x0, y0, x1, y1) = if y0 < y1 { (1, x0, y0, x1, y1) } else { (-1, x1, y1, x0, y0) };
        let (top, bottom) = (y0.max(0), y1.min(self.h as i32 * FX));
        if top >= bottom { return; }
        let x_at = |y: i32| x0 + ((x1 - x0) as i64 * (y - y0) as i64 / (y1 - y0) as i64) as i32;
        let mut row = top / FX;
        while row * FX < bottom {
            let (ya, yb) = (top.max(row * FX), bottom.min(row * FX + FX));
            if yb > ya { self.span(row as usize, x_at(ya), x_at(yb), yb - ya, dir); }
            row += 1;
        }
    }

    /// The part of an edge inside one pixel row: `dy` of height, from `xa` to `xb`.
    fn span(&mut self, row: usize, xa: i32, xb: i32, dy: i32, dir: i32) {
        let right_edge = self.w as i32 * FX;
        // Left of the bitmap counts as its first column, right of it as past the last:
        // only the horizontal position of an edge matters to the running sum.
        let (xa, xb) = (xa.clamp(0, right_edge), xb.clamp(0, right_edge));
        let (lo, hi) = if xa <= xb { (xa, xb) } else { (xb, xa) };
        let base = row * (self.w + 2);
        let first = lo / FX;
        let last = if hi > lo { (hi - 1) / FX } else { first };
        let mut deposit = |col: i32, l: i32, r: i32, height: i32| {
            // A trapezoid in one column: its area splits between this cell and the
            // next by where its middle sits.
            let d = dir * height * (ONE / FX);
            let mid = (l + r) / 2 - col * FX;
            let right = d * mid / FX;
            self.acc[base + col as usize] += d - right;
            self.acc[base + col as usize + 1] += right;
        };
        if first == last {
            deposit(first, lo, hi, dy);
            return;
        }
        // Across columns, the height divides in proportion to the width in each; the
        // last piece takes the remainder, so every edge deposits exactly `dy`.
        let (width, mut used) = ((hi - lo) as i64, 0);
        for col in first..=last {
            let (l, r) = (lo.max(col * FX), hi.min(col * FX + FX));
            let height = if col == last { dy - used } else { (dy as i64 * (r - l) as i64 / width) as i32 };
            used += height;
            deposit(col, l, r, height);
        }
    }

    /// A quadratic Bézier, flattened into enough lines to stay within a fraction of a pixel.
    pub fn quad(&mut self, p0: (i32, i32), p1: (i32, i32), p2: (i32, i32)) {
        let dd = ((p0.0 - 2 * p1.0 + p2.0) as i64, (p0.1 - 2 * p1.1 + p2.1) as i64);
        let n = segments(dd.0 * dd.0 + dd.1 * dd.1);
        let mut prev = p0;
        for i in 1..=n {
            let (a, b) = ((n - i) as i64, i as i64);
            let nn = (n as i64) * (n as i64);
            let at = |v0: i32, v1: i32, v2: i32| {
                div_round(a * a * v0 as i64 + 2 * a * b * v1 as i64 + b * b * v2 as i64, nn) as i32
            };
            let next = (at(p0.0, p1.0, p2.0), at(p0.1, p1.1, p2.1));
            self.line(prev.0, prev.1, next.0, next.1);
            prev = next;
        }
    }

    /// A cubic Bézier (CFF and variable outlines), flattened likewise.
    pub fn cubic(&mut self, p0: (i32, i32), p1: (i32, i32), p2: (i32, i32), p3: (i32, i32)) {
        let d = |a: (i32, i32), b: (i32, i32), c: (i32, i32)| {
            let (x, y) = ((a.0 - 2 * b.0 + c.0) as i64, (a.1 - 2 * b.1 + c.1) as i64);
            x * x + y * y
        };
        // A cubic bends up to 3/2 as far as its control polygon's second differences.
        let n = segments(d(p0, p1, p2).max(d(p1, p2, p3)) * 9 / 4);
        let mut prev = p0;
        let n3 = (n as i64).pow(3);
        for i in 1..=n {
            let (a, b) = ((n - i) as i64, i as i64);
            let at = |v0: i32, v1: i32, v2: i32, v3: i32| {
                div_round(a * a * a * v0 as i64 + 3 * a * a * b * v1 as i64 + 3 * a * b * b * v2 as i64 + b * b * b * v3 as i64, n3) as i32
            };
            let next = (at(p0.0, p1.0, p2.0, p3.0), at(p0.1, p1.1, p2.1, p3.1));
            self.line(prev.0, prev.1, next.0, next.1);
            prev = next;
        }
    }

    /// Coverage, 0 to 255, for each of the `w * h` pixels, row by row.
    pub fn finish(&self, out: &mut [u8]) {
        for y in 0..self.h {
            let mut sum = 0i32;
            let row = &self.acc[y * (self.w + 2)..];
            for x in 0..self.w {
                sum += row[x];
                let cover = sum.unsigned_abs().min(ONE as u32);
                out[y * self.w + x] = ((cover * 255 + ONE as u32 / 2) / ONE as u32) as u8;
            }
        }
    }
}

/// Lines for a curve whose second difference has squared length `devsq` (1/256 px
/// units): the deviation from a chord falls with the square of the segment count,
/// so a quarter-power keeps the error near a tenth of a pixel.
fn segments(devsq: i64) -> i32 {
    let px2 = devsq / (FX as i64 * FX as i64);
    (1 + isqrt(isqrt(3 * px2))).min(64) as i32
}

fn isqrt(v: i64) -> i64 {
    if v <= 0 { return 0; }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + v / x) / 2; }
    x
}

/// Division rounding half away from zero.
pub fn div_round(n: i64, d: i64) -> i64 {
    if (n < 0) != (d < 0) { (n - d / 2) / d } else { (n + d / 2) / d }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: i32, y0: i32, x1: i32, y1: i32, w: usize, h: usize) -> [u8; 64] {
        let mut acc = [0; scratch_len(8, 8)];
        let mut r = Raster::new(w, h, &mut acc).unwrap();
        r.line(x0, y0, x1, y0); r.line(x1, y0, x1, y1); r.line(x1, y1, x0, y1); r.line(x0, y1, x0, y0);
        let mut out = [0; 64];
        r.finish(&mut out[..w * h]);
        out
    }

    #[test]
    fn whole_pixel_square_is_solid_and_nothing_else() {
        let out = square(2 * FX, 2 * FX, 5 * FX, 6 * FX, 8, 8);
        for y in 0..8 { for x in 0..8 {
            let inside = (2..5).contains(&x) && (2..6).contains(&y);
            assert_eq!(out[y * 8 + x], if inside { 255 } else { 0 }, "{x},{y}");
        }}
    }

    #[test]
    fn half_pixel_edges_are_half_covered() {
        let out = square(FX / 2, FX / 2, 3 * FX + FX / 2, 2 * FX, 8, 8);
        assert_eq!(out[0], 64); // a quarter of the corner pixel
        assert_eq!(out[1], 128); // top edge: half
        assert_eq!(out[8], 128); // left edge: half
        assert_eq!(out[9], 255);
        assert_eq!(out[3], 64); // top right corner
        assert_eq!(out[8 + 3], 128); // right edge
        assert_eq!(out[8 + 4], 0);
    }

    #[test]
    fn winding_direction_does_not_matter_and_clipping_is_safe() {
        let a = square(FX, FX, 4 * FX, 4 * FX, 8, 8);
        let b = square(4 * FX, 4 * FX, FX, FX, 8, 8);
        assert_eq!(a, b);
        // Far outside on every side: clamped, no panic, the visible part is solid.
        let c = square(-40 * FX, -40 * FX, 40 * FX, 40 * FX, 8, 8);
        assert!(c.iter().all(|v| *v == 255));
    }

    #[test]
    fn diagonal_triangle_covers_half_the_box() {
        let mut acc = [0; scratch_len(8, 8)];
        let mut r = Raster::new(8, 8, &mut acc).unwrap();
        r.line(0, 0, 8 * FX, 8 * FX); r.line(8 * FX, 8 * FX, 0, 8 * FX); r.line(0, 8 * FX, 0, 0);
        let mut out = [0; 64];
        r.finish(&mut out);
        let total: u32 = out.iter().map(|v| *v as u32).sum();
        assert!((total as i32 - 32 * 255).abs() <= 8, "{total}");
        for i in 0..8 { assert_eq!(out[i * 8 + i], 128); }
    }
}
