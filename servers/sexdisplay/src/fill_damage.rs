//! Inline fill state changes after display has attested the actual surface owner.
//! Empty dimensions clear one rectangle. Old and new footprints bound all pixels
//! whose painter-order fill color can change; geometry and focus stay structural.
use crate::surface_lifecycle::{Surface, MAX_RECTS};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    const EMPTY: Self = Self {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };
    pub fn screen(self, s: &Surface, width: usize, height: usize) -> (usize, usize, usize, usize) {
        let left = s.x as i64 + self.x as i64;
        let top = s.y as i64 + self.y as i64;
        let x = left.clamp(0, width as i64) as usize;
        let y = top.clamp(0, height as i64) as usize;
        let right = (left + self.w as i64).clamp(0, width as i64) as usize;
        let bottom = (top + self.h as i64).clamp(0, height as i64) as usize;
        (x, y, right.saturating_sub(x), bottom.saturating_sub(y))
    }
}
/// None means no visible state change. Invalid indices have no effects.
pub fn update(
    s: &mut Surface,
    index: usize,
    sx: i32,
    sy: i32,
    sw: u32,
    sh: u32,
    color: u32,
) -> Result<Option<(Rect, Rect)>, ()> {
    if index >= MAX_RECTS {
        return Err(());
    }
    let old = if index < s.fill_count as usize {
        Rect {
            x: s.fill_sx[index],
            y: s.fill_sy[index],
            w: s.fill_sw[index],
            h: s.fill_sh[index],
        }
    } else {
        Rect::EMPTY
    };
    let old_color = s.fill_color[index];
    let (w, h) = (sw.min(s.w), sh.min(s.h));
    let new = if w == 0 || h == 0 {
        Rect::EMPTY
    } else {
        Rect {
            x: sx.clamp(0, s.w.saturating_sub(w).min(i32::MAX as u32) as i32),
            y: sy.clamp(0, s.h.saturating_sub(h).min(i32::MAX as u32) as i32),
            w,
            h,
        }
    };
    let color = if new == Rect::EMPTY {
        0
    } else {
        color & 0x00ff_ffff
    };
    if old == new && (new == Rect::EMPTY || old_color == color) {
        return Ok(None);
    }
    s.fill_sx[index] = new.x;
    s.fill_sy[index] = new.y;
    s.fill_sw[index] = new.w;
    s.fill_sh[index] = new.h;
    s.fill_color[index] = color;
    s.fill_count = s.fill_count.max((index + 1) as u8);
    while s.fill_count > 0 {
        let last = s.fill_count as usize - 1;
        if s.fill_sw[last] != 0 && s.fill_sh[last] != 0 {
            break;
        }
        s.fill_count -= 1;
    }
    Ok(Some((old, new)))
}
