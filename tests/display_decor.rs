// SPDX-License-Identifier: GPL-3.0-only
//! Display's real decoration pass (servers/sexdisplay/src/decor.rs) over a
//! flat reference compositor: rounded corners show what is under a window,
//! shadows fall where the stacking says, partial row redraws match full ones,
//! and the plain look leaves every pixel alone. Set DECOR_PREVIEW=path.ppm to
//! write a picture of the test desktop.
#![allow(dead_code)]
extern crate self as sex_graphics;
#[path = "../crates/sex-graphics/src/window_shape.rs"]
pub mod window_shape;

pub const MAX_FB_W: usize = 1920;

mod surface_lifecycle {
    pub const MAX_SURFACES: usize = 32;
    #[derive(Clone, Copy, Default)]
    pub struct Surface { pub active: bool, pub surface_id: u64, pub owner_pd: u32, pub x: i32, pub y: i32, pub w: u32, pub h: u32, pub color: u32 }
}
mod damage {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Rect { pub x: usize, pub y: usize, pub right: usize, pub bottom: usize }
}
mod wallpaper_mark {
    pub fn span(_: usize, _: usize, _: usize) -> Option<(usize, usize)> { None }
    pub fn pixel(_: usize, _: usize, _: usize, _: usize, back: u32) -> u32 { back }
}
mod scene {
    use crate::surface_lifecycle::{Surface, MAX_SURFACES};
    #[derive(Clone, Copy)]
    pub struct Metrics(pub u32);
    impl Metrics { pub fn scale(&self) -> u32 { self.0 } pub fn bar(&self) -> u32 { 28 * self.0 } }
    pub struct Groups;
    impl Groups {
        pub fn raised_focus(&self, _: &[Surface], focused: u64) -> u64 { focused }
        pub fn independent(&self, s: &Surface) -> bool { s.active }
        /// Production clamp (surface_groups::rect_below): origin clamped, extent kept.
        pub fn rect(&self, s: &Surface, w: usize, h: usize) -> (usize, usize, usize, usize) {
            let x = (s.x.max(0) as usize).min(w.saturating_sub(1));
            let y = (s.y.max(crate::BAR as i32) as usize).min(h.saturating_sub(1));
            (x, y, (s.w as usize).min(w.saturating_sub(x)), (s.h as usize).min(h.saturating_sub(y)))
        }
    }
    pub struct SceneSnapshot { pub surfaces: [Surface; MAX_SURFACES], pub groups: Groups, pub metrics: Metrics, pub focus: u64 }
}
#[path = "../servers/sexdisplay/src/decor.rs"]
mod decor;

use scene::SceneSnapshot;
use surface_lifecycle::Surface;

const W: usize = 640;
const H: usize = 400;
const BAR: usize = 28;
const TITLE: i32 = 26;

fn bg(y: usize, h: usize) -> u32 {
    let t = (y * 255 / h.max(1)) as u32;
    let ch = |a: u32, b: u32| (a * (255 - t) + b * t) / 255;
    (ch(0x33, 0x1C) << 16) | (ch(0x2E, 0x19) << 8) | ch(0x2F, 0x19)
}
/// Window colour at a pixel: a title bar on top of a body.
fn paint(s: &Surface, y: usize) -> u32 { if (y as i32) < s.y + TITLE { 0x00_1E1E2E } else { s.color } }
fn holds(s: &Surface, x: usize, y: usize) -> bool {
    let (sx, sy, sw, sh) = scene::Groups.rect(s, W, H);
    s.active && x >= sx && y >= sy && x < sx + sw && y < sy + sh
}
/// Focused first, then slot order: the production winner rule.
fn winner(scene: &SceneSnapshot, x: usize, y: usize) -> Option<&Surface> {
    scene.surfaces.iter().find(|s| s.surface_id == scene.focus && holds(s, x, y))
        .or_else(|| scene.surfaces.iter().find(|s| s.surface_id != scene.focus && holds(s, x, y)))
}
fn composite_pixel_selected(x: usize, y: usize, _w: usize, _h: usize, back: u32, _focus: u64, selected: Option<u64>, scene: &SceneSnapshot) -> u32 {
    scene.surfaces.iter().find(|s| s.active && Some(s.surface_id) == selected && holds(s, x, y)).map_or(back, |s| paint(s, y))
}
fn raster(scene: &SceneSnapshot, fb: &mut [u32], y: usize, x0: usize, x1: usize) {
    for x in x0..x1 { fb[y * W + x] = winner(scene, x, y).map_or(bg(y, H), |s| paint(s, y)); }
}
fn render(scene: &SceneSnapshot) -> Vec<u32> {
    let mut fb = vec![0u32; W * H];
    for y in BAR..H { raster(scene, &mut fb, y, 0, W); decor::row(fb.as_mut_ptr(), y, 0, W, W, H, scene); }
    fb
}
fn win(id: u64, x: i32, y: i32, w: u32, h: u32, color: u32) -> Surface {
    Surface { active: true, surface_id: id, owner_pd: 20 + id as u32, x, y, w, h, color }
}
/// Two tiles side by side (6 px gap) and a floating focused window over them.
fn desktop() -> SceneSnapshot {
    let mut surfaces = [Surface::default(); surface_lifecycle::MAX_SURFACES];
    surfaces[0] = win(1, 8, 36, 305, 356, 0x00_2A2B3C);
    surfaces[1] = win(2, 319, 36, 313, 356, 0x00_303446);
    surfaces[2] = win(3, 180, 120, 280, 180, 0x00_F5E0DC);
    SceneSnapshot { surfaces, groups: scene::Groups, metrics: scene::Metrics(1), focus: 3 }
}
fn dark(c: u32) -> u32 { ((c >> 16) & 255) + ((c >> 8) & 255) + (c & 255) }

#[test]
fn corners_show_the_wallpaper_and_bodies_are_untouched() {
    let _g = lock();
    decor::set_effects(true);
    let scene = desktop();
    let fb = render(&scene);
    // Top-left pixel of the left tile is wallpaper (a little shaded), not the title bar.
    let corner = fb[36 * W + 8];
    assert_ne!(corner, 0x00_1E1E2E);
    assert!(dark(corner) <= dark(bg(36, H)) && dark(corner) + 60 > dark(bg(36, H)), "corner {corner:06x}");
    // Middle of a body and a title bar are exactly the window.
    assert_eq!(fb[300 * W + 60], 0x00_2A2B3C);
    assert_eq!(fb[40 * W + 100], 0x00_1E1E2E);
    assert_eq!(fb[200 * W + 300], 0x00_F5E0DC);
}

#[test]
fn focused_window_casts_onto_windows_below_and_rests_only_on_wallpaper() {
    let _g = lock();
    decor::set_effects(true);
    let scene = desktop();
    let fb = render(&scene);
    // Just under the floating window, over the left tile: darker than the tile.
    let under = fb[(120 + 180 + 4) * W + 250];
    assert!(dark(under) < dark(0x00_2A2B3C), "shadow over tile {under:06x}");
    // Far from the floating window the tile is untouched.
    assert_eq!(fb[380 * W + 30], 0x00_2A2B3C);
    // The gap between the tiles is wallpaper with both resting shadows.
    let gap = fb[60 * W + 315];
    assert!(dark(gap) < dark(bg(60, H)), "gap shadow {gap:06x}");
    // The focused window's corner shows the tile beneath (blended), not the wallpaper.
    let fc = fb[120 * W + 180];
    assert!(dark(fc) < dark(0x00_F5E0DC));
}

#[test]
fn partial_row_redraws_match_a_full_frame() {
    let _g = lock();
    decor::set_effects(true);
    let scene = desktop();
    let full = render(&scene);
    let mut part = vec![0u32; W * H];
    for y in BAR..H {
        let mut x = 0;
        for step in [7usize, 64, 1, 130, 33, 200, 205] {
            let end = (x + step).min(W);
            raster(&scene, &mut part, y, x, end);
            decor::row(part.as_mut_ptr(), y, x, end, W, H, &scene);
            x = end;
        }
        assert_eq!(x, W);
    }
    for i in BAR * W..W * H { assert_eq!(full[i], part[i], "pixel {} {}", i % W, i / W); }
}

#[test]
fn plain_look_leaves_every_pixel_alone() {
    let _g = lock();
    let scene = desktop();
    assert!(decor::set_effects(false));
    let plain = render(&scene);
    decor::set_effects(true);
    for y in BAR..H { for x in 0..W { assert_eq!(plain[y * W + x], winner(&scene, x, y).map_or(bg(y, H), |s| paint(s, y))); } }
    let r = damage::Rect { x: 100, y: 100, right: 120, bottom: 110 };
    assert_eq!(decor::grow(r, W, H, BAR, 1), damage::Rect { x: 100 - decor::margin(1), y: 100 - decor::margin(1), right: 120 + decor::margin(1), bottom: 110 + decor::margin(1) });
    decor::set_effects(false);
    assert_eq!(decor::grow(r, W, H, BAR, 1), r);
    decor::set_effects(true);
}

#[test]
fn a_screen_filling_window_stays_square_and_casts_nothing() {
    let _g = lock();
    decor::set_effects(true);
    let mut scene = desktop();
    scene.surfaces = [Surface::default(); surface_lifecycle::MAX_SURFACES];
    scene.surfaces[0] = win(9, 0, BAR as i32, W as u32, (H - BAR) as u32, 0x00_445566);
    scene.focus = 9;
    let fb = render(&scene);
    assert_eq!(fb[BAR * W], 0x00_1E1E2E);
    assert_eq!(fb[(H - 1) * W + W - 1], 0x00_445566);
}

#[test]
fn stacked_corners_show_the_wallpaper_not_the_window_below() {
    let _g = lock();
    decor::set_effects(true);
    let mut scene = desktop();
    scene.surfaces = [Surface::default(); surface_lifecycle::MAX_SURFACES];
    scene.surfaces[0] = win(1, 100, 100, 200, 150, 0x00_445566);
    scene.surfaces[1] = win(2, 100, 100, 260, 200, 0x00_778899);
    scene.focus = 0;
    let fb = render(&scene);
    let c = fb[100 * W + 100];
    assert_ne!(c, 0x00_1E1E2E, "lower window's square title corner shows through");
    assert!(dark(c) <= dark(bg(100, H)));
}

#[test]
fn off_screen_origin_uses_the_drawn_rectangle() {
    let _g = lock();
    decor::set_effects(true);
    let mut scene = desktop();
    scene.surfaces = [Surface::default(); surface_lifecycle::MAX_SURFACES];
    scene.surfaces[0] = win(1, -20, 100, 100, 100, 0x00_445566);
    scene.focus = 1;
    let fb = render(&scene);
    // Drawn across x=0..100: no shadow inside it at x=90, and the drawn corner is rounded.
    assert_eq!(fb[150 * W + 90], 0x00_445566);
    assert!(dark(fb[150 * W + 101]) < dark(bg(150, H)));
    assert_ne!(fb[199 * W + 99], 0x00_445566);
}

#[test]
fn preview() {
    let _g = lock();
    decor::set_effects(true);
    let Ok(path) = std::env::var("DECOR_PREVIEW") else { return };
    let fb = render(&desktop());
    let mut out = format!("P6\n{W} {H}\n255\n").into_bytes();
    for y in 0..H { for x in 0..W {
        let c = if y < BAR { 0x00_2A2526 } else { fb[y * W + x] };
        out.extend_from_slice(&[(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    } }
    std::fs::write(path, out).unwrap();
}

fn lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
