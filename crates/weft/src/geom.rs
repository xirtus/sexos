// SPDX-License-Identifier: GPL-3.0-only
//! Points and rectangles in layout units (logical pixels; the theme's scale turns them
//! into device pixels at paint time).

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point { pub x: f32, pub y: f32 }

impl Point {
    pub const fn new(x: f32, y: f32) -> Self { Self { x, y } }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self { Self { x, y, w, h } }
    pub fn right(&self) -> f32 { self.x + self.w }
    pub fn bottom(&self) -> f32 { self.y + self.h }
    pub fn is_empty(&self) -> bool { self.w <= 0.0 || self.h <= 0.0 }
    pub fn contains(&self, p: Point) -> bool { p.x >= self.x && p.y >= self.y && p.x < self.right() && p.y < self.bottom() }
    pub fn inset(&self, d: f32) -> Rect { Rect::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0)) }
    pub fn inset_xy(&self, dx: f32, dy: f32) -> Rect { Rect::new(self.x + dx, self.y + dy, (self.w - 2.0 * dx).max(0.0), (self.h - 2.0 * dy).max(0.0)) }
    pub fn offset(&self, dx: f32, dy: f32) -> Rect { Rect::new(self.x + dx, self.y + dy, self.w, self.h) }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        Rect::new(x, y, (r - x).max(0.0), (b - y).max(0.0))
    }
    pub fn intersects(&self, o: &Rect) -> bool { !self.intersect(o).is_empty() }
    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() { return *o; }
        if o.is_empty() { return *self; }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(x, y, self.right().max(o.right()) - x, self.bottom().max(o.bottom()) - y)
    }
    pub fn scale(&self, s: f32) -> Rect { Rect::new(self.x * s, self.y * s, self.w * s, self.h * s) }
    /// The smallest whole-pixel rectangle holding this one.
    pub fn round_out(&self) -> Rect {
        let x = libm::floorf(self.x);
        let y = libm::floorf(self.y);
        Rect::new(x, y, libm::ceilf(self.right()) - x, libm::ceilf(self.bottom()) - y)
    }
    pub fn center(&self) -> Point { Point::new(self.x + self.w / 2.0, self.y + self.h / 2.0) }
}
