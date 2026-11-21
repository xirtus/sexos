// SPDX-License-Identifier: GPL-3.0-only
//! Frosted glass behind the desktop's popups (slice S6b): pure functions.
//!
//! A menu or panel drawn over the desktop shows the pixels behind it blurred
//! and tinted with the theme's panel colour instead of a flat fill. The blur
//! is two box passes each way (close to a gaussian), radius `RADIUS` times the
//! screen scale; the blend is toward the tint at `ALPHA`/255. Then each pixel's
//! WCAG luminance is kept in the band where every ink drawn on the panel still
//! reads at 4.5:1 (`Params`): over a bright wallpaper a dark panel is pulled
//! darker, over a dark one a light panel lighter, toward its own tint.
//!
//! The pixels come through `Pixels`, so Display runs this on its frame and the
//! host tests on a plain buffer. Nothing here allocates: the caller lends the
//! scratch lines, and a panel whose blur does not fit them is not frosted.
//! Builds alone (`tests/glass_panels.rs`).

/// Blur radius at scale 1, in pixels.
pub const RADIUS: usize = 6;
/// Weight of the tint over the blurred pixels, out of 255.
pub const ALPHA: u8 = 200;
/// The largest screen scale Display draws at.
pub const MAX_SCALE: u32 = 3;
/// Widest panel interior frosted; wider ones are drawn opaque.
pub const MAX_INNER_W: usize = 2048;
/// Two box passes of radius r read 2r pixels each way.
pub const fn margin(radius: usize) -> usize { 2 * radius }
/// The blur radius at screen scale `scale`.
pub const fn radius(scale: u32) -> usize {
    let s = if scale == 0 { 1 } else if scale > MAX_SCALE { MAX_SCALE } else { scale };
    RADIUS * s as usize
}
/// Rows above and below a panel kept between the passes, at most.
pub const BAND_LEN: usize = 2 * margin(radius(MAX_SCALE)) * MAX_INNER_W;

/// x, y, width, height.
pub type Rect = (usize, usize, usize, usize);

/// Why a panel is drawn opaque instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Off {
    /// The high-contrast theme: plain black and white, nothing behind.
    HighContrast,
    /// The person asked for Reduce transparency.
    Reduce,
    /// The panel and its blur margin do not fit the scratch lines.
    Oversize,
    /// Not even the tint itself reads with the inks (no glass can).
    Contrast,
}
impl Off {
    pub const fn reason(self) -> &'static str {
        match self { Off::HighContrast => "high_contrast", Off::Reduce => "reduce", Off::Oversize => "oversize", Off::Contrast => "contrast" }
    }
}

/// How a panel's glass is drawn: the tint, its weight, and the luminance band
/// (WCAG relative luminance times 10000) every frosted pixel stays in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params { pub tint: u32, pub alpha: u8, pub floor: u32, pub ceil: u32 }

/// sRGB channel to linear light, times 10000 (WCAG's formula, rounded).
const LINEAR: [u16; 256] = [
    0, 3, 6, 9, 12, 15, 18, 21, 24, 27, 30, 33, 37, 40, 44, 48,
    52, 56, 60, 65, 70, 75, 80, 86, 91, 97, 103, 110, 116, 123, 130, 137,
    144, 152, 160, 168, 176, 185, 194, 203, 212, 222, 232, 242, 252, 262, 273, 284,
    296, 307, 319, 331, 343, 356, 369, 382, 395, 409, 423, 437, 452, 467, 482, 497,
    513, 529, 545, 561, 578, 595, 612, 630, 648, 666, 685, 704, 723, 742, 762, 782,
    802, 823, 844, 865, 887, 908, 931, 953, 976, 999, 1022, 1046, 1070, 1095, 1119, 1144,
    1170, 1195, 1221, 1248, 1274, 1301, 1329, 1356, 1384, 1413, 1441, 1470, 1500, 1529, 1559, 1590,
    1620, 1651, 1683, 1714, 1746, 1779, 1812, 1845, 1878, 1912, 1946, 1981, 2016, 2051, 2086, 2122,
    2159, 2195, 2232, 2270, 2307, 2346, 2384, 2423, 2462, 2502, 2542, 2582, 2623, 2664, 2705, 2747,
    2789, 2831, 2874, 2918, 2961, 3005, 3050, 3095, 3140, 3185, 3231, 3278, 3325, 3372, 3419, 3467,
    3515, 3564, 3613, 3663, 3712, 3763, 3813, 3864, 3916, 3968, 4020, 4072, 4125, 4179, 4233, 4287,
    4342, 4397, 4452, 4508, 4564, 4621, 4678, 4735, 4793, 4851, 4910, 4969, 5029, 5089, 5149, 5210,
    5271, 5333, 5395, 5457, 5520, 5583, 5647, 5711, 5776, 5841, 5906, 5972, 6038, 6105, 6172, 6240,
    6308, 6376, 6445, 6514, 6584, 6654, 6724, 6795, 6867, 6939, 7011, 7084, 7157, 7231, 7305, 7379,
    7454, 7529, 7605, 7682, 7758, 7835, 7913, 7991, 8070, 8148, 8228, 8308, 8388, 8469, 8550, 8632,
    8714, 8796, 8879, 8963, 9047, 9131, 9216, 9301, 9387, 9473, 9560, 9647, 9734, 9823, 9911, 10000,
];

/// WCAG relative luminance of an XRGB colour, times 10000.
pub fn luminance(c: u32) -> u32 {
    let l = |shift: u32| LINEAR[((c >> shift) & 0xFF) as usize] as u32;
    (2126 * l(16) + 7152 * l(8) + 722 * l(0) + 5000) / 10000
}

/// Room left for the table's rounding (luminance units), so the band holds
/// with WCAG's exact formula too.
const SLACK: u32 = 30;

/// The glass for panels tinted `tint` that carry text in `inks`; Err when it is
/// off. An ink darker than the tint needs every pixel at least so bright, a
/// lighter one at most so dark, for 4.5:1: (light + 0.05) >= 4.5 (dark + 0.05).
pub fn params(tint: u32, inks: &[u32], high_contrast: bool, reduce: bool) -> Result<Params, Off> {
    if high_contrast { return Err(Off::HighContrast); }
    if reduce { return Err(Off::Reduce); }
    let tint = tint & 0x00FF_FFFF;
    let lt = luminance(tint);
    let (mut floor, mut ceil) = (0u32, 10000u32);
    for &ink in inks {
        let li = luminance(ink);
        if li <= lt {
            // 10 * (L + 500) >= 45 * (li + 500)
            let need = (45 * (li + 500) - 5000 + 9) / 10 + SLACK;
            floor = floor.max(need);
        } else {
            // 45 * (L + 500) <= 10 * (li + 500)
            let most = (10 * (li + 500) / 45).saturating_sub(500 + SLACK);
            ceil = ceil.min(most);
        }
    }
    if floor > ceil || lt < floor || lt > ceil { return Err(Off::Contrast); }
    Ok(Params { tint, alpha: ALPHA, floor, ceil })
}

/// `a` toward `b` by `t`/255, each channel rounded.
pub fn mix(a: u32, b: u32, t: u8) -> u32 {
    let (t, u) = (t as u32, 255 - t as u32);
    let m = |s: u32| ((((a >> s) & 0xFF) * u + ((b >> s) & 0xFF) * t + 127) / 255) << s;
    m(16) | m(8) | m(0)
}
/// `a` toward `b` by `k`/256 (k 0..=256), for the luminance search.
fn mix256(a: u32, b: u32, k: u32) -> u32 {
    let m = |s: u32| ((((a >> s) & 0xFF) * (256 - k) + ((b >> s) & 0xFF) * k + 128) >> 8) << s;
    m(16) | m(8) | m(0)
}

/// One blurred pixel made glass: blended toward the tint, then, if its
/// luminance left the band, moved further toward the tint (whose luminance is
/// in the band) by the least of 32 steps that brings it back.
pub fn frost_pixel(blurred: u32, p: &Params) -> u32 {
    let q = mix(blurred, p.tint, p.alpha);
    let ok = |c: u32| { let l = luminance(c); p.floor <= l && l <= p.ceil };
    if ok(q) { return q; }
    let (mut bad, mut good) = (0u32, 256u32);
    while good - bad > 8 {
        let mid = (bad + good) / 2;
        if ok(mix256(q, p.tint, mid)) { good = mid } else { bad = mid }
    }
    mix256(q, p.tint, good)
}

/// One box pass of radius `r` over a line, the ends repeated past the edges.
/// Each channel is the rounded mean of the 2r+1 pixels around it.
pub fn box_line(src: &[u32], dst: &mut [u32], r: usize) {
    let n = src.len().min(dst.len());
    if n == 0 { return; }
    let d = 2 * r as u32 + 1;
    let at = |i: isize| src[i.clamp(0, n as isize - 1) as usize];
    let (mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32);
    for i in -(r as isize)..=(r as isize) {
        let c = at(i);
        sr += (c >> 16) & 0xFF; sg += (c >> 8) & 0xFF; sb += c & 0xFF;
    }
    for x in 0..n {
        dst[x] = ((sr + d / 2) / d) << 16 | ((sg + d / 2) / d) << 8 | (sb + d / 2) / d;
        let (inn, out) = (at(x as isize + r as isize + 1), at(x as isize - r as isize));
        sr = sr + ((inn >> 16) & 0xFF) - ((out >> 16) & 0xFF);
        sg = sg + ((inn >> 8) & 0xFF) - ((out >> 8) & 0xFF);
        sb = sb + (inn & 0xFF) - (out & 0xFF);
    }
}

/// The frame the glass reads and writes.
pub trait Pixels {
    fn get(&self, x: usize, y: usize) -> u32;
    fn set(&mut self, x: usize, y: usize, c: u32);
}

/// Lent scratch: the rows above and below the panel between the passes, and
/// two lines as long as the longest row or column of the blur.
pub struct Scratch<'a> { pub band: &'a mut [u32], pub a: &'a mut [u32], pub b: &'a mut [u32] }

/// The pixels the blur of `inner` reads: grown by `margin` each way, inside
/// `bounds` (x0, y0, x1, y1). As (x0, y0, x1, y1).
pub fn region(inner: Rect, margin: usize, bounds: (usize, usize, usize, usize)) -> (usize, usize, usize, usize) {
    let (x, y, w, h) = inner;
    (x.saturating_sub(margin).max(bounds.0), y.saturating_sub(margin).max(bounds.1),
     (x + w + margin).min(bounds.2), (y + h + margin).min(bounds.3))
}

/// `inner` cut to `bounds` (x0, y0, x1, y1).
pub fn clip(inner: Rect, bounds: (usize, usize, usize, usize)) -> Rect {
    let (x0, y0) = (inner.0.max(bounds.0), inner.1.max(bounds.1));
    let (x1, y1) = ((inner.0 + inner.2).min(bounds.2), (inner.1 + inner.3).min(bounds.3));
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

/// True when the blur of `inner` (already inside `bounds`) fits `s`.
pub fn fits(inner: Rect, r: usize, bounds: (usize, usize, usize, usize), s: &Scratch<'_>) -> bool {
    let (x, y, w, h) = inner;
    let (rx0, ry0, rx1, ry1) = region(inner, margin(r), bounds);
    let band = ((y - ry0) + (ry1 - (y + h))) * w;
    let line = (rx1 - rx0).max(ry1 - ry0);
    x >= rx0 && w <= MAX_INNER_W && band <= s.band.len() && line <= s.a.len() && line <= s.b.len()
}

/// Frosts `inner` in place: every pixel in it becomes the glass over what the
/// frame held there and around it (the blur reads `margin(r)` beyond the
/// panel, inside `bounds`). Err(Oversize), with nothing written, when the
/// scratch is too small or the panel reaches outside `bounds` (over the bar).
pub fn frost(px: &mut dyn Pixels, inner: Rect, r: usize, bounds: (usize, usize, usize, usize), p: &Params, s: &mut Scratch<'_>) -> Result<(), Off> {
    let (x0, y0, w, h) = inner;
    if w == 0 || h == 0 { return Ok(()); }
    if clip(inner, bounds) != inner || !fits(inner, r, bounds, s) { return Err(Off::Oversize); }
    let (x1, y1) = (x0 + w, y0 + h);
    let (rx0, ry0, rx1, ry1) = region(inner, margin(r), bounds);
    let above = y0 - ry0;
    let band_row = |y: usize| if y < y0 { y - ry0 } else { above + (y - y1) };
    // Across: each row of the region, blurred; the panel's own rows go back
    // into the frame (they are overwritten below), the others into the band.
    let n = rx1 - rx0;
    for y in ry0..ry1 {
        for i in 0..n { s.a[i] = px.get(rx0 + i, y); }
        box_line(&s.a[..n], &mut s.b[..n], r);
        box_line(&s.b[..n], &mut s.a[..n], r);
        let row = &s.a[x0 - rx0..x1 - rx0];
        if y0 <= y && y < y1 {
            for (i, &c) in row.iter().enumerate() { px.set(x0 + i, y, c); }
        } else {
            let at = band_row(y) * w;
            s.band[at..at + w].copy_from_slice(row);
        }
    }
    // Down: each column of the panel, then the glass.
    let n = ry1 - ry0;
    for x in x0..x1 {
        for (i, y) in (ry0..ry1).enumerate() {
            s.a[i] = if y0 <= y && y < y1 { px.get(x, y) } else { s.band[band_row(y) * w + (x - x0)] };
        }
        box_line(&s.a[..n], &mut s.b[..n], r);
        box_line(&s.b[..n], &mut s.a[..n], r);
        for y in y0..y1 { px.set(x, y, frost_pixel(s.a[y - ry0], p)); }
    }
    Ok(())
}
