// SPDX-License-Identifier: GPL-3.0-only
//! The lips on the SexOS wallpaper (`sex_pdx::brand`).
//!
//! The mark's coverage is worked out once per screen size and kept here; every
//! background pixel inside its box is then the gradient with the rose-gold
//! blended over it. Wallpapers other than the SexOS one show no mark.

use sex_pdx::brand;

/// Widest mark kept: 26% of a 3840 px screen fits; wider screens get this size.
const CAP_W: usize = 1024;
const CAP_H: usize = brand::height_for(CAP_W as u32) as usize;
static mut COVER: [u8; CAP_W * CAP_H] = [0; CAP_W * CAP_H];
/// (screen w, screen h) the cache was made for, and the mark's (x, y, w, h).
static mut KEY: (usize, usize) = (0, 0);
static mut PLACE: (usize, usize, usize, usize) = (0, 0, 0, 0);

/// Where the mark sits on a `w` x `h` screen, or None when it is not shown.
#[allow(static_mut_refs)]
fn place(w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    if !sex_pdx::desktop_look::shows_mark() || w < 64 || h < 64 { return None; }
    unsafe {
        if KEY != (w, h) {
            let (_, _, want) = brand::wallpaper_place(w as u32, h as u32);
            let mw = (want as usize).min(CAP_W);
            let mh = brand::height_for(mw as u32) as usize;
            let x = (w - mw) / 2;
            let y = (h * 48 / 100).saturating_sub(mh / 2);
            for row in 0..mh {
                brand::coverage_row(mw as u32, row as u32, &mut COVER[row * mw..(row + 1) * mw]);
            }
            KEY = (w, h);
            PLACE = (x, y, mw, mh);
        }
        Some(PLACE)
    }
}

/// The columns [x0, x1) of row `y` the mark's box covers, if any.
/// Desktop widgets (`desktop_widgets.rs`) count as part of the wallpaper here.
pub fn span(y: usize, w: usize, h: usize) -> Option<(usize, usize)> {
    let mark = place(w, h).and_then(|(x, top, mw, mh)| (y >= top && y < top + mh).then_some((x, x + mw)));
    match (mark, super::desktop_widgets::span(y)) {
        (Some(a), Some(b)) => Some((a.0.min(b.0), a.1.max(b.1))),
        (a, b) => a.or(b),
    }
}

/// The desktop at (x, y): `back`, the gradient's colour for the row, with the
/// mark over it where the mark is.
#[inline]
pub fn pixel(x: usize, y: usize, w: usize, h: usize, back: u32) -> u32 {
    super::desktop_widgets::over(x, y, mark_pixel(x, y, w, h, back))
}

#[inline]
fn mark_pixel(x: usize, y: usize, w: usize, h: usize, back: u32) -> u32 {
    let Some((left, top, mw, mh)) = place(w, h) else { return back };
    if x < left || y < top || x >= left + mw || y >= top + mh { return back; }
    let c = unsafe { COVER[(y - top) * mw + (x - left)] };
    if c == 0 { return back; }
    brand::blend(brand::color_at(brand::Tone::Rich, (x - left) as u32, mw as u32), back, c)
}
