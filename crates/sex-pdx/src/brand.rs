// SPDX-License-Identifier: GPL-3.0-only
//! The SexOS mark: rose-gold lips.
//!
//! The vector original is `assets/brand/sexos-lips.svg`; `scripts/brand/lips.py`
//! writes both it and the flattened outline in `brand_lips.rs`, so the logo the
//! kernel, Display and the Shell draw is the same shape as the SVG. Drawing is a
//! scanline fill with four sub-rows per pixel and exact horizontal coverage, in
//! integers only (the kernel has no floating point), into a row the caller owns:
//! nothing here allocates, so any size can be drawn from any domain.

mod lips {
    include!("brand_lips.rs");
}
pub use lips::{BRIGHT, CHARCOAL, DESIGN_H, DESIGN_W, RICH, WALL_BOTTOM, WALL_TOP};

/// Which gradient the mark wears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// The prototype's own: dark at the left corner, peach at the right. For
    /// sizes where the mark is the subject (boot screen, wallpaper, about).
    Rich,
    /// Lighter throughout, for small sizes on dark glass (the Silkbar button),
    /// where the dark corner would disappear.
    Bright,
}

/// The mark's height for a given width, keeping its proportions.
pub const fn height_for(width: u32) -> u32 {
    (width * DESIGN_H + DESIGN_W / 2) / DESIGN_W
}
/// The widest width that fits a `w` x `h` box.
pub const fn fit_width(w: u32, h: u32) -> u32 {
    let by_height = (h * DESIGN_W) / DESIGN_H;
    if by_height < w { by_height } else { w }
}

/// Coverage (0 none, 255 full) of row `y` of the mark drawn `width` pixels wide,
/// written to `out[0..width]`. Columns past `out.len()` are not drawn.
pub fn coverage_row(width: u32, y: u32, out: &mut [u8]) {
    let n = (width as usize).min(out.len());
    out[..n].fill(0);
    if width == 0 || y >= height_for(width) { return; }
    // Pixel space in 1/256 px: design units are 1/16, so 16 * width / DESIGN_W.
    let k = width as i64 * 16;
    let d = DESIGN_W as i64;
    for sub in 0..4i64 {
        let sy = y as i64 * 256 + 32 + sub * 64;
        let mut xs = [0i64; 32];
        let mut count = 0;
        for poly in [&lips::UPPER[..], &lips::LOWER[..]] {
            for i in 0..poly.len() {
                let (ax, ay) = poly[i];
                let (bx, by) = poly[(i + 1) % poly.len()];
                let (ax, ay, bx, by) = (ax as i64 * k / d, ay as i64 * k / d, bx as i64 * k / d, by as i64 * k / d);
                if (ay <= sy) == (by <= sy) || count == xs.len() { continue; }
                xs[count] = ax + (sy - ay) * (bx - ax) / (by - ay);
                count += 1;
            }
        }
        let xs = &mut xs[..count];
        xs.sort_unstable();
        // The two lips never overlap, so even-odd pairs are the filled spans.
        for span in xs.chunks_exact(2) {
            let (a, b) = (span[0].max(0), span[1].min(n as i64 * 256));
            if b <= a { continue; }
            let (first, last) = ((a >> 8) as usize, ((b - 1) >> 8) as usize);
            for (px, slot) in out.iter_mut().enumerate().take(last + 1).skip(first) {
                let lo = a.max(px as i64 * 256);
                let hi = b.min(px as i64 * 256 + 256);
                *slot = (*slot as i64 + (hi - lo) / 4).min(255) as u8;
            }
        }
    }
}

/// The gradient's colour (0xRRGGBB) at column `x` of a mark `width` pixels wide.
pub fn color_at(tone: Tone, x: u32, width: u32) -> u32 {
    let stops: &[(u32, u32)] = match tone { Tone::Rich => &RICH, Tone::Bright => &BRIGHT };
    let t = if width <= 1 { 0 } else { (x.min(width - 1) * 255) / (width - 1) };
    let mut i = 0;
    while i + 2 < stops.len() && t > stops[i + 1].0 { i += 1; }
    let ((t0, c0), (t1, c1)) = (stops[i], stops[i + 1]);
    let f = if t1 > t0 { ((t.clamp(t0, t1) - t0) * 256) / (t1 - t0) } else { 0 };
    mix(c1, c0, f.min(256))
}

/// `fg` over `bg` by `coverage` (0..=255); both 0xRRGGBB, the top byte of `bg` kept.
pub fn blend(fg: u32, bg: u32, coverage: u8) -> u32 {
    match coverage {
        0 => bg,
        255 => (bg & 0xFF00_0000) | (fg & 0x00FF_FFFF),
        c => (bg & 0xFF00_0000) | mix(fg, bg, c as u32 + 1),
    }
}

/// `a` weighted `w`/256 against `b`, per channel.
fn mix(a: u32, b: u32, w: u32) -> u32 {
    let ch = |s: u32| ((((a >> s) & 0xFF) * w + ((b >> s) & 0xFF) * (256 - w)) >> 8) << s;
    ch(16) | ch(8) | ch(0)
}

/// Draws the whole mark `width` pixels wide by calling `put(x, y, color)` for
/// every pixel it touches, with `color` already blended over `under(x, y)`.
/// Widths past 2048 pixels are drawn 2048 wide.
pub fn draw(width: u32, tone: Tone, mut under: impl FnMut(u32, u32) -> u32, mut put: impl FnMut(u32, u32, u32)) {
    let width = width.min(2048);
    let mut row = [0u8; 2048];
    for y in 0..height_for(width) {
        coverage_row(width, y, &mut row);
        for x in 0..width {
            let c = row[x as usize];
            if c != 0 { put(x, y, blend(color_at(tone, x, width), under(x, y), c)); }
        }
    }
}

/// The desktop wallpaper's mark: 26% of the screen's width, centred, its middle
/// at 48% of the height. Returns (left, top, width).
pub const fn wallpaper_place(screen_w: u32, screen_h: u32) -> (u32, u32, u32) {
    let w = screen_w * 26 / 100;
    let h = height_for(w);
    let top = (screen_h * 48 / 100).saturating_sub(h / 2);
    ((screen_w - w) / 2, top, w)
}
