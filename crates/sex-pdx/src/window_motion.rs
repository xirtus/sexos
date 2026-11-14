// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Window animations: a window that opens rises into place, a window the
//! tiler moves or resizes glides there, and on a workspace switch the new
//! workspace's windows slide in from the side they come from. Display keeps
//! the tweens and draws each frame at its tweened geometry; the Shell only
//! turns animations on or off and says which way a workspace switch goes.
//!
//! Shell -> Display `OP_MOTION`: arg0 = 1 animations on, 0 off;
//! arg1 = workspace slide direction as i8 (-1 from the left, 1 from the
//! right, 0 none) for the windows shown next.

pub const OP_MOTION: u64 = 0x2E5;
/// How long a tween runs, in ticks (ms).
pub const DURATION: u64 = 180;
/// How far an opening window rises.
pub const RISE: i32 = 40;
/// Opens within this many ticks of a workspace switch slide in from the side.
pub const SLIDE_WINDOW: u64 = 300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geom { pub x: i32, pub y: i32, pub w: u32, pub h: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tween { surface: u64, from: Geom, to: Geom, start: u64 }

/// Ease-out cubic in 1/1024 steps: fast start, gentle landing.
pub fn ease(elapsed: u64, duration: u64) -> u32 {
    if duration == 0 || elapsed >= duration { return 1024; }
    let t = (elapsed * 1024 / duration) as i64;
    let u = 1024 - t;
    (1024 - u * u * u / (1024 * 1024)) as u32
}
fn lerp(a: i64, b: i64, k: u32) -> i64 { a + (b - a) * k as i64 / 1024 }
impl Tween {
    fn at(&self, now: u64) -> Option<Geom> {
        let e = now.saturating_sub(self.start);
        if e >= DURATION { return None; }
        let k = ease(e, DURATION);
        let (f, t) = (self.from, self.to);
        Some(Geom {
            x: lerp(f.x as i64, t.x as i64, k) as i32,
            y: lerp(f.y as i64, t.y as i64, k) as i32,
            w: lerp(f.w as i64, t.w as i64, k).max(1) as u32,
            h: lerp(f.h as i64, t.h as i64, k).max(1) as u32,
        })
    }
}

pub struct Motion<const N: usize> {
    tweens: [Option<Tween>; N],
    enabled: bool,
    slide: i8,
    slide_at: u64,
    width: i32,
}

impl<const N: usize> Motion<N> {
    pub const fn new() -> Self { Self { tweens: [None; N], enabled: true, slide: 0, slide_at: 0, width: 0 } }
    pub fn enabled(&self) -> bool { self.enabled }
    /// Off stops every tween: windows jump to where they belong.
    pub fn set_enabled(&mut self, on: bool) { self.enabled = on; if !on { self.tweens = [None; N]; } }
    /// The next windows shown come from the left (-1) or right (1), `width` away.
    pub fn workspace(&mut self, dir: i8, width: u32, now: u64) { self.slide = dir.signum(); self.slide_at = now; self.width = width as i32; }
    /// Where `surface` is drawn now, if it is moving.
    pub fn current(&self, surface: u64, now: u64) -> Option<Geom> {
        self.tweens.iter().flatten().find(|t| t.surface == surface).and_then(|t| t.at(now))
    }
    fn start(&mut self, surface: u64, from: Geom, to: Geom, now: u64) {
        if !self.enabled || from == to { self.forget(surface); return; }
        let tween = Tween { surface, from, to, start: now };
        let slot = self.tweens.iter().position(|t| t.is_some_and(|t| t.surface == surface))
            .or_else(|| self.tweens.iter().position(|t| t.is_none_or(|t| t.at(now).is_none())))
            // Full: the oldest gives way.
            .or_else(|| (0..N).min_by_key(|&i| self.tweens[i].map_or(0, |t| t.start)));
        if let Some(i) = slot { self.tweens[i] = Some(tween); }
    }
    /// A window was moved or resized from `from` to `to`; a moving window
    /// continues from where it is drawn now.
    pub fn moved(&mut self, surface: u64, from: Geom, to: Geom, now: u64) {
        let from = self.current(surface, now).unwrap_or(from);
        self.start(surface, from, to, now);
    }
    /// A window appeared at `to`: it rises into place, or slides in after a
    /// workspace switch.
    pub fn opened(&mut self, surface: u64, to: Geom, now: u64) {
        let slide = self.slide != 0 && now.saturating_sub(self.slide_at) < SLIDE_WINDOW;
        let from = if slide { Geom { x: to.x + self.slide as i32 * self.width, ..to } } else { Geom { y: to.y + RISE, ..to } };
        self.start(surface, from, to, now);
    }
    pub fn forget(&mut self, surface: u64) {
        for t in self.tweens.iter_mut() { if t.is_some_and(|t| t.surface == surface) { *t = None; } }
    }
    /// Every place a tween (running or just finished) passes through, as
    /// (left, top, right, bottom): the damage to redraw for its frames.
    pub fn span(&self) -> Option<(i32, i32, i32, i32)> {
        self.tweens.iter().flatten().fold(None, |acc, t| {
            let r = (t.from.x.min(t.to.x), t.from.y.min(t.to.y),
                (t.from.x + t.from.w as i32).max(t.to.x + t.to.w as i32), (t.from.y + t.from.h as i32).max(t.to.y + t.to.h as i32));
            Some(match acc { None => r, Some(a) => (a.0.min(r.0), a.1.min(r.1), a.2.max(r.2), a.3.max(r.3)) })
        })
    }
    /// Drops finished tweens once their last frame is drawn.
    pub fn clear_finished(&mut self, now: u64) {
        for t in self.tweens.iter_mut() { if t.is_some_and(|t| t.at(now).is_none()) { *t = None; } }
    }
    /// Progress of the least advanced running tween, 0..=1024.
    pub fn progress(&self, now: u64) -> Option<u32> {
        self.tweens.iter().flatten().filter(|t| t.at(now).is_some()).map(|t| ease(now.saturating_sub(t.start), DURATION)).min()
    }
    /// True while any window is still moving: Display draws another frame.
    pub fn active(&self, now: u64) -> bool { self.tweens.iter().flatten().any(|t| t.at(now).is_some()) }
}
