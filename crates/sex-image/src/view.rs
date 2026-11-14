// SPDX-License-Identifier: GPL-3.0-only
//! Looking at a decoded picture: quarter turns, zoom steps and a panned window onto a
//! picture larger than the screen, plus which file names are pictures at all.
//!
//! Pure functions over `0xAARRGGBB` pixel slices, with no allocation the caller does
//! not hand in, so a viewer can reuse one scratch buffer for every frame of an
//! animation and the host tests can check each pixel's destination.

/// Writes `src` (`w` by `h`) turned clockwise by `turns` quarter turns into `dst`, and
/// returns the new width and height. `dst` must hold `w * h` pixels.
pub fn rotate_into(src: &[u32], (w, h): (usize, usize), turns: u8, dst: &mut [u32]) -> (usize, usize) {
    let turns = turns % 4;
    let (ow, oh) = if turns % 2 == 1 { (h, w) } else { (w, h) };
    for y in 0..h {
        for x in 0..w {
            // Where the source pixel (x, y) lands after the turn.
            let (nx, ny) = match turns {
                0 => (x, y),
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            dst[ny * ow + nx] = src[y * w + x];
        }
    }
    (ow, oh)
}

/// The steps + and - walk, in percent.
const LADDER: [u32; 14] = [10, 25, 33, 50, 67, 75, 100, 125, 150, 200, 300, 400, 600, 800];

/// The percentage at which a `w` by `h` picture fits an `aw` by `ah` area: 100 when it
/// already fits (pictures are never enlarged to fit), otherwise the tighter ratio.
pub fn fit_percent((w, h): (usize, usize), (aw, ah): (usize, usize)) -> u32 {
    if w <= aw && h <= ah { return 100; }
    let by_w = aw * 100 / w.max(1);
    let by_h = ah * 100 / h.max(1);
    by_w.min(by_h).max(1) as u32
}

/// The next zoom step above `pct`.
pub fn zoom_in(pct: u32) -> u32 { LADDER.iter().copied().find(|p| *p > pct).unwrap_or(LADDER[LADDER.len() - 1]) }

/// The next zoom step below `pct`.
pub fn zoom_out(pct: u32) -> u32 { LADDER.iter().rev().copied().find(|p| *p < pct).unwrap_or(LADDER[0]) }

/// The picture's size at `pct` percent, at least one pixel each way.
pub fn scaled((w, h): (usize, usize), pct: u32) -> (usize, usize) {
    ((w * pct as usize / 100).max(1), (h * pct as usize / 100).max(1))
}

/// Keeps a pan offset (the scaled picture's pixel at the area's top left) inside the
/// picture: no further than its far edge less the area, and none on an axis that fits.
pub fn clamp_pan(pan: (isize, isize), (sw, sh): (usize, usize), (aw, ah): (usize, usize)) -> (usize, usize) {
    let most = |s: usize, a: usize| s.saturating_sub(a) as isize;
    (pan.0.clamp(0, most(sw, aw)) as usize, pan.1.clamp(0, most(sh, ah)) as usize)
}

/// Draws the part of `src` (`w` by `h`) at `pct` percent that an `aw` by `ah` area
/// shows from `pan` (from [`clamp_pan`]) into `out`, nearest pixel, rows `ow` wide.
/// Returns the size drawn (`ow`, `oh`): the area, or less where the picture is smaller.
pub fn view_into(src: &[u32], (w, h): (usize, usize), pct: u32, pan: (usize, usize), out: &mut [u32], (aw, ah): (usize, usize)) -> (usize, usize) {
    let (sw, sh) = scaled((w, h), pct);
    let (ow, oh) = (sw.min(aw), sh.min(ah));
    for oy in 0..oh {
        let sy = ((pan.1 + oy) * h / sh).min(h - 1);
        let row = &src[sy * w..sy * w + w];
        for ox in 0..ow {
            let sx = ((pan.0 + ox) * w / sw).min(w - 1);
            out[oy * ow + ox] = row[sx];
        }
    }
    (ow, oh)
}

/// Whether a file name ends in an extension View opens, in any case.
pub fn is_picture_name(name: &[u8]) -> bool {
    let Some(dot) = name.iter().rposition(|b| *b == b'.') else { return false };
    let ext = &name[dot + 1..];
    [&b"png"[..], b"jpg", b"jpeg", b"webp", b"gif"].iter().any(|e| ext.eq_ignore_ascii_case(e))
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec;

    // 3 by 2:  0 1 2
    //          3 4 5
    const SRC: [u32; 6] = [0, 1, 2, 3, 4, 5];

    #[test]
    fn quarter_turns_move_each_pixel() {
        let mut dst = [9u32; 6];
        assert_eq!(rotate_into(&SRC, (3, 2), 1, &mut dst), (2, 3));
        // Clockwise: the left column, read bottom up, becomes the top row.
        assert_eq!(dst, [3, 0, 4, 1, 5, 2]);
        assert_eq!(rotate_into(&SRC, (3, 2), 2, &mut dst), (3, 2));
        assert_eq!(dst, [5, 4, 3, 2, 1, 0]);
        assert_eq!(rotate_into(&SRC, (3, 2), 3, &mut dst), (2, 3));
        assert_eq!(dst, [2, 5, 1, 4, 0, 3]);
        assert_eq!(rotate_into(&SRC, (3, 2), 4, &mut dst), (3, 2));
        assert_eq!(dst, SRC);
    }

    #[test]
    fn zoom_steps_and_fit() {
        assert_eq!(fit_percent((300, 200), (640, 400)), 100);
        assert_eq!(fit_percent((1200, 400), (600, 400)), 50);
        assert_eq!(fit_percent((400, 1600), (600, 400)), 25);
        assert_eq!(zoom_in(100), 125);
        assert_eq!(zoom_in(57), 67);
        assert_eq!(zoom_out(100), 75);
        assert_eq!(zoom_out(10), 10);
        assert_eq!(zoom_in(800), 800);
        assert_eq!(scaled((300, 200), 200), (600, 400));
    }

    #[test]
    fn panned_window_shows_the_right_pixels() {
        // At 200 % the 3 by 2 picture is 6 by 4; a 2 by 2 area from (2, 2) sees the
        // bottom-middle source pixel four times.
        let mut out = vec![0u32; 4];
        let pan = clamp_pan((2, 2), scaled((3, 2), 200), (2, 2));
        assert_eq!(pan, (2, 2));
        assert_eq!(view_into(&SRC, (3, 2), 200, pan, &mut out, (2, 2)), (2, 2));
        assert_eq!(out, [4, 4, 4, 4]);
        // Pans beyond the far edge stop there; an axis that fits never pans.
        assert_eq!(clamp_pan((50, -5), (6, 4), (2, 8)), (4, 0));
        // A picture smaller than the area is drawn at its own scaled size.
        let mut all = vec![0u32; 24];
        assert_eq!(view_into(&SRC, (3, 2), 200, (0, 0), &mut all, (10, 10)), (6, 4));
        assert_eq!(&all[..6], &[0, 0, 1, 1, 2, 2]);
    }

    #[test]
    fn picture_names() {
        assert!(is_picture_name(b"a.PNG"));
        assert!(is_picture_name(b"x.y.webp"));
        assert!(is_picture_name(b"photo.JPeG"));
        assert!(!is_picture_name(b"notes.txt"));
        assert!(!is_picture_name(b"png"));
    }
}
