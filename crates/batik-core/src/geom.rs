// SPDX-License-Identifier: GPL-3.0-only
//! 2D geometry primitives for Batik canvas operations.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: usize, height: usize) -> Self {
        Self { x, y, width, height }
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn right(&self) -> i32 {
        let w = if self.width > (i32::MAX as usize) {
            i32::MAX
        } else {
            self.width as i32
        };
        self.x.saturating_add(w)
    }

    pub const fn bottom(&self) -> i32 {
        let h = if self.height > (i32::MAX as usize) {
            i32::MAX
        } else {
            self.height as i32
        };
        self.y.saturating_add(h)
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());

        if x2 > x1 && y2 > y1 {
            Some(Rect {
                x: x1,
                y: y1,
                width: (x2 - x1) as usize,
                height: (y2 - y1) as usize,
            })
        } else {
            None
        }
    }

    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x1 = self.x.min(other.x);
        let y1 = self.y.min(other.y);
        let x2 = self.right().max(other.right());
        let y2 = self.bottom().max(other.bottom());
        Rect {
            x: x1,
            y: y1,
            width: (x2 - x1) as usize,
            height: (y2 - y1) as usize,
        }
    }

    pub fn clamp_to_bounds(&self, bounds_w: usize, bounds_h: usize) -> Option<Rect> {
        self.intersect(&Rect::new(0, 0, bounds_w, bounds_h))
    }
}
