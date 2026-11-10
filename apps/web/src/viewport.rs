//! Private pixel geometry. Authority comes from Window's held grant and its
//! authenticated resize events, never from page script arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub w: i32,
    pub h: i32,
    pub pixels: usize,
}
impl Surface {
    pub const EMPTY: Self = Self {
        w: 0,
        h: 0,
        pixels: 0,
    };
    pub fn checked(w: usize, h: usize, capacity: usize) -> Option<Self> {
        if w == 0 || h == 0 {
            return Some(Self::EMPTY);
        }
        // Geometry/raster code uses signed coordinates and products. Reject an
        // impossible notice rather than truncating it into another viewport.
        let pixels = w.checked_mul(h)?;
        if w > i32::MAX as usize
            || h > i32::MAX as usize
            || pixels > capacity
            || pixels > i32::MAX as usize
        {
            return None;
        }
        Some(Self {
            w: w as i32,
            h: h as i32,
            pixels,
        })
    }
    pub fn document(self, bar: i32) -> (i32, i32) {
        (self.w, self.h.saturating_sub(bar).max(0))
    }
    pub fn empty(self) -> bool {
        self.pixels == 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub source: Surface,
    pub sx: usize,
    pub sy: usize,
    pub dx: usize,
    pub dy: usize,
    pub w: usize,
    pub h: usize,
}
impl Frame {
    /// A frame's scratch must fit the same bounded grant budget. Wide coordinate
    /// arithmetic clips negative/offscreen boxes without signed overflow.
    pub fn visible(
        surface: Surface,
        rect: [i32; 4],
        scroll: i32,
        bar: i32,
        capacity: usize,
    ) -> Option<Self> {
        let [x, y, w, h] = rect;
        if surface.empty() || w <= 0 || h <= 0 {
            return None;
        }
        let source = Surface::checked(w as usize, h as usize, capacity)?;
        let (x, y) = (x as i64, y as i64 - scroll as i64 + bar as i64);
        let left = x.max(0);
        let top = y.max(bar.max(0) as i64);
        let right = (x + w as i64).min(surface.w as i64);
        let bottom = (y + h as i64).min(surface.h as i64);
        if left >= right || top >= bottom {
            return None;
        }
        Some(Self {
            source,
            sx: (left - x) as usize,
            sy: (top - y) as usize,
            dx: left as usize,
            dy: top as usize,
            w: (right - left) as usize,
            h: (bottom - top) as usize,
        })
    }
    pub fn copy(self, surface: Surface, destination: &mut [u32], source: &[u32]) -> bool {
        if destination.len() < surface.pixels || source.len() < self.source.pixels {
            return false;
        }
        for row in 0..self.h {
            let from = (self.sy + row) * self.source.w as usize + self.sx;
            let to = (self.dy + row) * surface.w as usize + self.dx;
            destination[to..to + self.w].copy_from_slice(&source[from..from + self.w]);
        }
        true
    }
}
