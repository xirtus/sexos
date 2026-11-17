// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Window shape: rounded corners and soft drop shadows.
//!
//! Display draws with these numbers and the Shell hit-tests with the same
//! ones, so a click on the see-through corner of a window falls to whatever
//! is drawn there. Everything is integer math in 1/16 pixel steps: no floats,
//! no tables, the same answer on every core.

/// Corner radius in logical pixels (times the UI scale on screen).
pub const RADIUS: u32 = 10;

/// A drop shadow: how far it reaches past the window, how far it is pushed
/// down, and how dark it is right under the window edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shadow { pub spread: u32, pub offset_y: i32, pub alpha: u8 }

/// The focused window floats above the others with a deep, soft shadow.
pub const FOCUSED: Shadow = Shadow { spread: 28, offset_y: 8, alpha: 150 };
/// Other windows get a short ambient shadow on the wallpaper.
pub const RESTING: Shadow = Shadow { spread: 10, offset_y: 2, alpha: 80 };

impl Shadow {
    /// The same shadow at a UI scale.
    pub const fn scaled(self, scale: u32) -> Self {
        Self { spread: self.spread * scale, offset_y: self.offset_y * scale as i32, alpha: self.alpha }
    }
    /// How far past the window rectangle this shadow can darken anything.
    pub const fn reach(&self) -> u32 { self.spread + self.offset_y.unsigned_abs() }
}

/// Corner radius for a `w` x `h` window at `scale`: never more than half a side.
pub fn radius(scale: u32, w: u32, h: u32) -> u32 { (RADIUS * scale.max(1)).min(w / 2).min(h / 2) }

/// Integer square root (floor).
pub fn isqrt(v: u64) -> u64 {
    if v < 2 { return v; }
    let mut x = v;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + v / x) / 2; }
    x
}

/// Distance of the corner centre from the pixel centre along one axis, in
/// 1/16 px, or None when the pixel is not in a corner band on that axis.
fn corner_axis(l: i32, size: u32, r: u32) -> Option<i64> {
    let (l, size, r) = (l as i64, size as i64, r as i64);
    let centre = l * 16 + 8;
    if l < r { Some(r * 16 - centre) } else if l >= size - r { Some(centre - (size - r) * 16) } else { None }
}

/// How much of pixel (`lx`, `ly`) a `w` x `h` window with corner radius `r`
/// covers: 255 inside, 0 outside, in between on the anti-aliased arc.
pub fn coverage(lx: i32, ly: i32, w: u32, h: u32, r: u32) -> u8 {
    if lx < 0 || ly < 0 || lx >= w as i32 || ly >= h as i32 { return 0; }
    if r == 0 { return 255; }
    let (Some(dx), Some(dy)) = (corner_axis(lx, w, r), corner_axis(ly, h, r)) else { return 255; };
    let d = isqrt((dx * dx + dy * dy) as u64) as i64;
    let c = (r as i64 * 16 - d + 8).clamp(0, 16);
    (c * 255 / 16) as u8
}

/// Pixels at each end of row `ly` that are not fully covered (0 outside the
/// corner rows). Rows are symmetric, so one count serves both ends.
pub fn row_inset(ly: i32, w: u32, h: u32, r: u32) -> u32 {
    if r == 0 || ly < 0 || ly >= h as i32 { return 0; }
    if corner_axis(ly, h, r).is_none() { return 0; }
    (0..r).find(|&x| coverage(x as i32, ly, w, h, r) == 255).unwrap_or(r)
}

/// Hit mask: true when a click at (`lx`, `ly`) belongs to the window.
pub fn contains(lx: i32, ly: i32, w: u32, h: u32, r: u32) -> bool { coverage(lx, ly, w, h, r) >= 128 }

/// Signed distance in 1/16 px from pixel centre (`lx`, `ly`) to a rounded
/// rectangle `w` x `h` (radius `r`) whose top-left is at (0, `top`).
fn rounded_distance(lx: i32, ly: i32, w: u32, h: u32, r: u32, top: i32) -> i64 {
    let (px, py) = (lx as i64 * 16 + 8, (ly - top) as i64 * 16 + 8);
    let (hw, hh, r16) = (w as i64 * 8, h as i64 * 8, r as i64 * 16);
    let qx = (px - hw).abs() - hw + r16;
    let qy = (py - hh).abs() - hh + r16;
    let (ox, oy) = (qx.max(0), qy.max(0));
    isqrt((ox * ox + oy * oy) as u64) as i64 + qx.max(qy).min(0) - r16
}

/// How dark `shadow` is at (`lx`, `ly`) relative to the window's top-left.
/// Callers apply it only to pixels the window does not cover itself.
pub fn shadow_alpha(lx: i32, ly: i32, w: u32, h: u32, r: u32, shadow: Shadow) -> u8 {
    if shadow.spread == 0 || shadow.alpha == 0 || w == 0 || h == 0 { return 0; }
    let d = rounded_distance(lx, ly, w, h, r, shadow.offset_y);
    let spread = shadow.spread as i64 * 16;
    if d <= 0 { return shadow.alpha; }
    if d >= spread { return 0; }
    // Quadratic fall-off: dark at the edge, a long faint tail.
    let t = (spread - d) * 1024 / spread;
    (shadow.alpha as i64 * t * t / (1024 * 1024)) as u8
}

/// Stack two shadows (each lets some light through).
pub fn stack(a: u8, b: u8) -> u8 { (255 - (255 - a as u32) * (255 - b as u32) / 255) as u8 }

/// Darken an XRGB pixel by `alpha` toward black.
pub fn darken(c: u32, alpha: u8) -> u32 {
    if alpha == 0 { return c; }
    let keep = 255 - alpha as u32;
    let ch = |s: u32| (((c >> s) & 0xFF) * keep / 255) << s;
    (c & 0xFF00_0000) | ch(16) | ch(8) | ch(0)
}

/// `fg` over `bg` with `coverage` (255 = all `fg`).
pub fn blend(fg: u32, bg: u32, coverage: u8) -> u32 {
    if coverage == 255 { return fg; }
    if coverage == 0 { return bg; }
    let (a, inv) = (coverage as u32, 255 - coverage as u32);
    let ch = |s: u32| ((((fg >> s) & 0xFF) * a + ((bg >> s) & 0xFF) * inv) / 255) << s;
    (fg & 0xFF00_0000) | ch(16) | ch(8) | ch(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_are_cut_and_edges_are_solid() {
        let (w, h, r) = (200, 100, 10);
        assert_eq!(coverage(0, 0, w, h, r), 0);
        assert_eq!(coverage(199, 0, w, h, r), 0);
        assert_eq!(coverage(0, 99, w, h, r), 0);
        assert_eq!(coverage(199, 99, w, h, r), 0);
        assert_eq!(coverage(100, 0, w, h, r), 255);
        assert_eq!(coverage(0, 50, w, h, r), 255);
        assert_eq!(coverage(10, 10, w, h, r), 255);
        assert_eq!(coverage(-1, 50, w, h, r), 0);
        // The arc is anti-aliased: some pixel on row 1 is partly covered.
        assert!((0..10).any(|x| { let c = coverage(x, 1, w, h, r); c > 0 && c < 255 }));
    }

    #[test]
    fn corners_are_symmetric() {
        let (w, h, r) = (64, 48, 12);
        for y in 0..h as i32 { for x in 0..w as i32 {
            let c = coverage(x, y, w, h, r);
            assert_eq!(c, coverage(w as i32 - 1 - x, y, w, h, r));
            assert_eq!(c, coverage(x, h as i32 - 1 - y, w, h, r));
        } }
    }

    #[test]
    fn row_inset_matches_coverage() {
        let (w, h, r) = (80, 60, 10);
        for y in 0..h as i32 {
            let n = row_inset(y, w, h, r);
            for x in 0..n as i32 { assert!(coverage(x, y, w, h, r) < 255); }
            assert_eq!(coverage(n as i32, y, w, h, r), 255);
        }
        assert_eq!(row_inset(30, w, h, r), 0);
        assert!(row_inset(0, w, h, r) >= 3);
    }

    #[test]
    fn hit_mask_drops_the_corner_tip_only() {
        assert!(!contains(0, 0, 100, 100, 10));
        assert!(contains(3, 3, 100, 100, 10));
        assert!(contains(0, 10, 100, 100, 10));
        assert!(contains(50, 0, 100, 100, 10));
    }

    #[test]
    fn radius_fits_small_windows() {
        assert_eq!(radius(1, 400, 300), 10);
        assert_eq!(radius(2, 400, 300), 20);
        assert_eq!(radius(1, 12, 300), 6);
        assert_eq!(radius(1, 0, 0), 0);
    }

    #[test]
    fn shadow_fades_with_distance_and_sits_lower() {
        let s = FOCUSED;
        let (w, h, r) = (200, 100, 10);
        let below = shadow_alpha(100, 100, w, h, r, s);
        let above = shadow_alpha(100, -1, w, h, r, s);
        assert!(below > above, "offset shadow is darker below ({below}) than above ({above})");
        let near = shadow_alpha(-2, 50, w, h, r, s);
        let far = shadow_alpha(-20, 50, w, h, r, s);
        assert!(near > far && far > 0);
        assert_eq!(shadow_alpha(-40, 50, w, h, r, s), 0);
        assert!(shadow_alpha(100, 100 + s.offset_y + s.spread as i32 + 1, w, h, r, s) == 0);
        // Monotonic outward on the left edge.
        let mut last = 255;
        for x in (-(s.spread as i32)..0).rev() { let a = shadow_alpha(x, 50, w, h, r, s); assert!(a <= last); last = a; }
    }

    #[test]
    fn colour_helpers() {
        assert_eq!(darken(0x00FF_FFFF, 0), 0x00FF_FFFF);
        assert_eq!(darken(0x00FF_FFFF, 255), 0);
        assert_eq!(blend(0x00FF_0000, 0x0000_00FF, 255), 0x00FF_0000);
        assert_eq!(blend(0x00FF_0000, 0x0000_00FF, 0), 0x0000_00FF);
        assert_eq!(stack(0, 100), 100);
        assert!(stack(100, 100) > 100);
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(15), 3);
        assert_eq!(isqrt(16), 4);
        assert_eq!(isqrt(1 << 40), 1 << 20);
    }
}
