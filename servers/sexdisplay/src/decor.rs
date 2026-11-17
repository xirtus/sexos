// SPDX-License-Identifier: GPL-3.0-only
//! Window decoration pass: rounded corners and soft drop shadows.
//!
//! `raster_row` paints each row with square windows first; this pass then
//! touches only the pixels near window edges. A corner pixel outside the
//! arc shows what is under the window (the next window or the wallpaper),
//! an arc pixel blends the two, and pixels around a window darken with its
//! shadow. The focused window is on top and shadows everything under it;
//! other windows shadow only the wallpaper (tiled windows sit side by side).
//! The shape numbers live in `sex_graphics::window_shape`, shared with the
//! Shell's hit masks.
//!
//! The pass reads only the immutable render scene, so a partial redraw of a
//! row gives the same pixels as a full one.

use core::sync::atomic::{AtomicBool, Ordering};
use sex_graphics::window_shape::{self as shape, Shadow};
use crate::surface_lifecycle::MAX_SURFACES;

/// Rounded corners and shadows on (the desktop look switch turns them off).
static EFFECTS: AtomicBool = AtomicBool::new(true);
pub fn set_effects(on: bool) -> bool { EFFECTS.swap(on, Ordering::Relaxed) != on }
pub fn effects() -> bool { EFFECTS.load(Ordering::Relaxed) }

/// Surfaces that are never decorated: the cursor and the Shell's modal
/// launcher (which draws its own panel on top of everything).
const CURSOR: u64 = 0x90;
const LAUNCHER_OVERLAY: u64 = 152;
const SHELL_PD: u32 = 3;

#[derive(Clone, Copy)]
struct Win { id: u64, x: i32, y: i32, w: u32, h: u32, r: u32, shadow: Shadow, focused: bool }
impl Win {
    const EMPTY: Self = Self { id: 0, x: 0, y: 0, w: 0, h: 0, r: 0, shadow: Shadow { spread: 0, offset_y: 0, alpha: 0 }, focused: false };
    fn holds(&self, x: i32, y: i32) -> bool { x >= self.x && y >= self.y && x < self.x + self.w as i32 && y < self.y + self.h as i32 }
    fn cover(&self, x: i32, y: i32) -> u8 { shape::coverage(x - self.x, y - self.y, self.w, self.h, self.r) }
    fn shade(&self, x: i32, y: i32) -> u8 { shape::shadow_alpha(x - self.x, y - self.y, self.w, self.h, self.r, self.shadow) }
}

/// Largest distance any decoration reaches past a window, at `scale`.
pub fn margin(scale: u32) -> usize { shape::FOCUSED.scaled(scale).reach() as usize + 1 }

/// A damage rectangle grown by the shadow reach, clipped below the bar.
pub fn grow(rect: crate::damage::Rect, w: usize, h: usize, bar: usize, scale: u32) -> crate::damage::Rect {
    if !effects() { return rect; }
    let m = margin(scale);
    crate::damage::Rect { x: rect.x.saturating_sub(m), y: rect.y.saturating_sub(m).max(bar),
        right: rect.right.saturating_add(m).min(w), bottom: rect.bottom.saturating_add(m).min(h) }
}

const WORDS: usize = crate::MAX_FB_W / 64;

/// Decorate framebuffer row `y` between `x0` and `x1`.
pub fn row(fb: *mut u32, y: usize, x0: usize, x1: usize, w: usize, h: usize, scene: &crate::scene::SceneSnapshot) {
    if !effects() || x0 >= x1 || x1 > w || w > crate::MAX_FB_W || y >= h { return; }
    let scale = scene.metrics.scale().max(1);
    let bar = scene.metrics.bar() as i32;
    let focus = scene.groups.raised_focus(&scene.surfaces, scene.focus);
    let (yi, x0i, x1i) = (y as i32, x0 as i32, x1 as i32);
    // Windows in winner order: the raised focus first, then slot order.
    let mut wins = [Win::EMPTY; MAX_SURFACES];
    let mut n = 0;
    let mut overlay: Option<Win> = None;
    for pass in 0..2 {
        for s in scene.surfaces.iter() {
            if !s.active || s.w == 0 || s.h == 0 || (s.surface_id == focus) != (pass == 0) { continue; }
            if s.surface_id == CURSOR { continue; }
            if !scene.groups.independent(s) { continue; }
            // The compositor's own clamped rectangle, so shape and pixels agree.
            let (rx, ry, rw, rh) = scene.groups.rect(s, w, h);
            if rw == 0 || rh == 0 { continue; }
            let (sx, sy, sw, sh) = (rx as i32, ry as i32, rw as u32, rh as u32);
            if s.owner_pd == SHELL_PD && s.surface_id == LAUNCHER_OVERLAY {
                overlay = Some(Win { id: s.surface_id, x: sx, y: sy, w: sw, h: sh, ..Win::EMPTY });
                continue;
            }
            let focused = pass == 0;
            // A window filling the screen keeps square corners and casts nothing.
            let full = sx <= 0 && sy <= bar && sx + sw as i32 >= w as i32 && sy + sh as i32 >= h as i32;
            let shadow = if full { Shadow { spread: 0, offset_y: 0, alpha: 0 } }
                else if focused { shape::FOCUSED.scaled(scale) } else { shape::RESTING.scaled(scale) };
            let reach = shadow.reach() as i32;
            if yi < sy - reach || yi >= sy + sh as i32 + reach { continue; }
            if n < MAX_SURFACES {
                wins[n] = Win { id: s.surface_id, x: sx, y: sy, w: sw, h: sh,
                    r: if full { 0 } else { shape::radius(scale, sw, sh) }, shadow, focused };
                n += 1;
            }
        }
    }
    if n == 0 { return; }
    let wins = &wins[..n];
    // Mark the pixels that may change: shadow bands and corner arcs.
    let mut mark = [0u64; WORDS];
    let mut any = false;
    let mut set = |a: i32, b: i32| {
        let (a, b) = (a.max(x0i) as usize, b.min(x1i).max(0) as usize);
        for x in a..b.max(a) { mark[x / 64] |= 1 << (x % 64); any = true; }
    };
    for v in wins {
        let reach = v.shadow.reach() as i32;
        let ly = yi - v.y;
        if ly < 0 || ly >= v.h as i32 {
            if v.shadow.alpha != 0 { set(v.x - reach, v.x + v.w as i32 + reach); }
        } else {
            let inset = shape::row_inset(ly, v.w, v.h, v.r) as i32;
            if v.shadow.alpha != 0 || inset != 0 {
                set(v.x - reach, v.x + inset);
                set(v.x + v.w as i32 - inset, v.x + v.w as i32 + reach);
            }
        }
    }
    if !any { return; }
    let back = crate::bg(y, h);
    let marked = crate::wallpaper_mark::span(y, w, h).is_some();
    let wall = |x: usize| if marked { crate::wallpaper_mark::pixel(x, y, w, h, back) } else { back };
    // Shadow on whatever lies under window `below` (None = wallpaper).
    let shade_under = |x: i32, below: Option<usize>| -> u8 {
        let mut a = 0u8;
        for (i, v) in wins.iter().enumerate() {
            if v.shadow.alpha == 0 || below == Some(i) { continue; }
            // Over a window only the focused one (on top) casts.
            let casts = match below { None => true, Some(b) => v.focused && i < b };
            if casts && v.cover(x, yi) == 0 { a = shape::stack(a, v.shade(x, yi)); }
        }
        a
    };
    let row_ptr = unsafe { fb.add(y * w) };
    for word in x0 / 64..=(x1 - 1) / 64 {
        let mut bits = mark[word];
        while bits != 0 {
            let x = word * 64 + bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if x < x0 || x >= x1 { continue; }
            let xi = x as i32;
            if overlay.is_some_and(|o| o.holds(xi, yi)) { continue; }
            let cur = unsafe { core::ptr::read_volatile(row_ptr.add(x)) };
            let top = wins.iter().position(|v| v.holds(xi, yi));
            let out = match top {
                None => shape::darken(cur, shade_under(xi, None)),
                Some(t) => {
                    let cover = wins[t].cover(xi, yi);
                    let over = if wins[t].focused { 0 } else { shade_under(xi, Some(t)) };
                    if cover == 255 { shape::darken(cur, over) } else {
                        // Down the stack: each window below with its own corner coverage.
                        let mut layers = [(0u32, 0u8); 4];
                        let mut depth = 0;
                        let mut from = t + 1;
                        let mut under = loop {
                            let Some(b) = wins[from..].iter().position(|v| v.holds(xi, yi)).map(|i| i + from) else {
                                break shape::darken(wall(x), shade_under(xi, None));
                            };
                            let c = shape::darken(crate::composite_pixel_selected(x, y, w, h, wall(x), scene.focus, Some(wins[b].id), scene), shade_under(xi, Some(b)));
                            let k = wins[b].cover(xi, yi);
                            if k == 255 || depth == layers.len() { break c; }
                            layers[depth] = (c, k); depth += 1; from = b + 1;
                        };
                        while depth > 0 { depth -= 1; let (c, k) = layers[depth]; under = shape::blend(c, under, k); }
                        shape::darken(shape::blend(cur, under, cover), over)
                    }
                }
            };
            if out != cur { unsafe { core::ptr::write_volatile(row_ptr.add(x), out); } }
        }
    }
}
