// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Several screens as one desktop: where each output sits in desktop
//! coordinates (logical pixels, after its scale), which output a point is on,
//! and where a window lands when it moves to another output.

use alloc::vec::Vec;
use crate::model::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    pub connector: u32,
    /// Physical pixels.
    pub width: u32,
    pub height: u32,
    /// Integer HiDPI scale (1, 2, 3); logical size is physical / scale.
    pub scale: u32,
    /// Top-left in desktop (logical) coordinates.
    pub x: i32,
    pub y: i32,
    pub internal: bool,
}
impl Output {
    pub fn logical(&self) -> Rect {
        let s = self.scale.max(1);
        Rect::new(self.x, self.y, self.width / s, self.height / s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Desktop { pub outputs: Vec<Output> }

impl Desktop {
    /// Internal panel first, then the others left to right, tops aligned.
    pub fn arrange(mut outputs: Vec<Output>) -> Self {
        outputs.sort_by_key(|o| (!o.internal, o.connector));
        let mut x = 0i32;
        for o in outputs.iter_mut() { o.x = x; o.y = 0; x += o.logical().w as i32; }
        Self { outputs }
    }
    pub fn bounds(&self) -> Rect {
        let mut it = self.outputs.iter().map(|o| o.logical());
        let Some(first) = it.next() else { return Rect::default() };
        let (mut l, mut t, mut r, mut b) = (first.x as i64, first.y as i64, first.right(), first.bottom());
        for o in it { l = l.min(o.x as i64); t = t.min(o.y as i64); r = r.max(o.right()); b = b.max(o.bottom()); }
        Rect::new(l as i32, t as i32, (r - l) as u32, (b - t) as u32)
    }
    pub fn output_at(&self, x: i32, y: i32) -> Option<usize> {
        self.outputs.iter().position(|o| o.logical().contains_point(x, y))
    }
    /// The output holding most of a window, for "which screen is it on".
    pub fn output_of(&self, r: &Rect) -> Option<usize> {
        let area = |o: &Rect| {
            let w = (r.right().min(o.right()) - (r.x as i64).max(o.x as i64)).max(0);
            let h = (r.bottom().min(o.bottom()) - (r.y as i64).max(o.y as i64)).max(0);
            w * h
        };
        let best = self.outputs.iter().enumerate().map(|(i, o)| (area(&o.logical()), i)).max()?;
        if best.0 > 0 { Some(best.1) } else { None }
    }
    /// Move a window to output `to`, keeping its relative position and
    /// clamping it inside the new screen.
    pub fn move_to(&self, r: &Rect, to: usize) -> Option<Rect> {
        let dst = self.outputs.get(to)?.logical();
        let src = self.output_of(r).map(|i| self.outputs[i].logical()).unwrap_or(dst);
        let rel = |v: i32, o: i32, s: u32, d: u32| -> i64 { if s == 0 { 0 } else { (v - o) as i64 * d as i64 / s as i64 } };
        let w = r.w.min(dst.w);
        let h = r.h.min(dst.h);
        let x = (dst.x as i64 + rel(r.x, src.x, src.w, dst.w)).clamp(dst.x as i64, dst.right() - w as i64);
        let y = (dst.y as i64 + rel(r.y, src.y, src.h, dst.h)).clamp(dst.y as i64, dst.bottom() - h as i64);
        Some(Rect::new(x as i32, y as i32, w, h))
    }
}
