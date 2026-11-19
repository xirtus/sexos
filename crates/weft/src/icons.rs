// SPDX-License-Identifier: GPL-3.0-only
//! Weft's icons: the SexOS icon set (`sex_icons`, from assets/icons/icons.txt), one
//! consistent line style on a 24-unit grid, drawn as vector paths at any scale
//! (1x, 1.25x, 2x look the same, only sharper).
//!
//! Path data is SVG path commands (`M L H V C Q Z`, upper and lower case) plus
//! `O cx cy r` for a circle, `D cx cy r` for a filled dot and `F` before a subpath
//! that is filled. Strokes are 2 units with round caps and joins. A program adds its
//! own icons with [`Icons::add`] in the same notation. App icons (rose gold on a
//! charcoal badge) draw with [`Icons::draw_app`].
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Stroke};

use crate::geom::Rect;
use crate::paint::Painter;
use crate::theme::Color;

/// The built-in line icons, by name.
pub fn builtin() -> impl Iterator<Item = (&'static str, &'static str)> {
    sex_icons::all().filter(|(k, _, _)| *k == sex_icons::Kind::Line).map(|(_, n, d)| (n, d))
}

#[derive(Clone, Copy, Debug)]
enum Part { Stroke, Fill }

/// The icon set: built-in icons plus any the program adds, parsed once.
pub struct Icons {
    parsed: BTreeMap<String, Vec<(Part, Path)>>,
}

impl Default for Icons {
    fn default() -> Self { Self::new() }
}

impl Icons {
    pub fn new() -> Icons {
        let mut icons = Icons { parsed: BTreeMap::new() };
        for (name, data) in builtin() { icons.add(name, data); }
        icons
    }
    /// Adds (or replaces) an icon in Weft's notation; returns false if it doesn't parse.
    pub fn add(&mut self, name: &str, data: &str) -> bool {
        match parse(data) {
            Some(parts) => { self.parsed.insert(name.into(), parts); true }
            None => false,
        }
    }
    pub fn has(&self, name: &str) -> bool { self.parsed.contains_key(name) }
    pub fn names(&self) -> impl Iterator<Item = &str> { self.parsed.keys().map(|s| s.as_str()) }

    /// Draws app icon `name` (a program name or alias: `files` is Loom) to fill `r`,
    /// in the brand's rose gold on a charcoal badge, at device resolution.
    pub fn draw_app(&self, p: &mut Painter, name: &str, r: Rect) -> bool {
        self.draw_kind(p, sex_icons::Kind::App, sex_icons::app_for(name).unwrap_or(name), r)
    }

    /// Draws the Files icon for a file `name` (or a folder) to fill `r`.
    pub fn draw_file(&self, p: &mut Painter, name: &[u8], dir: bool, r: Rect) -> bool {
        self.draw_kind(p, sex_icons::Kind::File, sex_icons::file_for(name, dir), r)
    }

    fn draw_kind(&self, p: &mut Painter, kind: sex_icons::Kind, name: &str, r: Rect) -> bool {
        let side = libm::roundf(r.w.min(r.h) * p.scale).max(1.0) as usize;
        let (a, m) = sex_icons::scratch(side);
        let (mut acc, mut mask, mut px) = (vec![0i32; a], vec![0u8; m], vec![0u32; side * side]);
        if !sex_icons::draw(kind, name, side, &sex_icons::Palette::BRAND, &mut acc, &mut mask, &mut px) { return false; }
        let s = r.w.min(r.h);
        p.image_alpha(Rect::new(r.x + (r.w - s) / 2.0, r.y + (r.h - s) / 2.0, s, s), side, side, &px);
        true
    }

    /// Draws icon `name` to fill `r` (square, layout units) in colour `c`.
    pub fn draw(&self, p: &mut Painter, name: &str, r: Rect, c: Color) -> bool {
        let Some(parts) = self.parsed.get(name) else { return false };
        let side = r.w.min(r.h);
        let k = side / 24.0;
        let (ox, oy) = (r.x + (r.w - side) / 2.0, r.y + (r.h - side) / 2.0);
        let mut paint = Paint::default();
        paint.set_color_rgba8(c.b(), c.g(), c.r(), c.a());
        paint.anti_alias = true;
        // Strokes are 2 grid units, but never thinner than 1.6 device pixels.
        let device = (2.0 * k * p.scale).max(1.6);
        let stroke = Stroke { width: device / (k * p.scale), line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        p.skia_draw(r, |pm, t, mask| {
            let t = t.pre_translate(ox, oy).pre_scale(k, k);
            for (part, path) in parts {
                match part {
                    Part::Stroke => pm.stroke_path(path, &paint, &stroke, t, mask),
                    Part::Fill => pm.fill_path(path, &paint, FillRule::Winding, t, mask),
                }
            }
        });
        true
    }
}

/// Path data to tiny-skia paths: one stroked path, filled subpaths and circles.
fn parse(data: &str) -> Option<Vec<(Part, Path)>> {
    struct Build { stroke: PathBuilder, fill: PathBuilder, filling: bool, parts: Vec<(Part, Path)>, bad: bool }
    impl Build { fn pb(&mut self) -> &mut PathBuilder { if self.filling { &mut self.fill } else { &mut self.stroke } } }
    impl sex_icons::Sink for Build {
        fn begin(&mut self, x: f32, y: f32, fill: bool) { self.filling = fill; self.pb().move_to(x, y); }
        fn line_to(&mut self, x: f32, y: f32) { self.pb().line_to(x, y); }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) { self.pb().quad_to(x1, y1, x, y); }
        fn cubic_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) { self.pb().cubic_to(x1, y1, x2, y2, x, y); }
        fn close(&mut self) { self.pb().close(); }
        fn circle(&mut self, cx: f32, cy: f32, r: f32, filled: bool) {
            match PathBuilder::from_circle(cx, cy, r) {
                Some(c) => self.parts.push((if filled { Part::Fill } else { Part::Stroke }, c)),
                None => self.bad = true,
            }
        }
    }
    let mut b = Build { stroke: PathBuilder::new(), fill: PathBuilder::new(), filling: false, parts: Vec::new(), bad: false };
    if !sex_icons::walk(data, &mut b) || b.bad { return None; }
    let Build { stroke, fill, mut parts, .. } = b;
    if let Some(path) = fill.finish() { parts.insert(0, (Part::Fill, path)); }
    if let Some(path) = stroke.finish() { parts.insert(0, (Part::Stroke, path)); }
    (!parts.is_empty()).then_some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_builtin_icon_parses_and_draws_at_1x_and_2x() {
        let icons = Icons::new();
        assert_eq!(icons.names().count(), builtin().count());
        for scale in [1.0f32, 2.0] {
            let n = (24.0 * scale) as usize;
            for (name, _) in builtin() {
                let mut px = alloc::vec![0xFFFFFFFFu32; n * n];
                let mut p = Painter::new(&mut px, n, n, scale);
                assert!(icons.draw(&mut p, name, Rect::new(0.0, 0.0, 24.0, 24.0), Color::hex(0)));
                let ink = px.iter().filter(|c| **c & 0xFF < 128).count();
                assert!(ink * 100 / (n * n) >= 2, "{name} at {scale}x has almost no ink ({ink})");
            }
        }
    }
    #[test]
    fn app_and_file_icons_draw_into_the_painter() {
        let icons = Icons::new();
        let mut px = alloc::vec![0xFF10_1010u32; 64 * 64];
        let mut p = Painter::new(&mut px, 64, 64, 2.0);
        assert!(icons.draw_app(&mut p, "files", Rect::new(0.0, 0.0, 32.0, 32.0)));
        assert!(!icons.draw_app(&mut p, "nonesuch", Rect::new(0.0, 0.0, 32.0, 32.0)));
        // The badge's rounded corner leaves the background; its middle is charcoal.
        assert_eq!(px[0], 0xFF10_1010);
        assert_ne!(px[32 * 64 + 4], 0xFF10_1010);
        let mut px = alloc::vec![0xFF10_1010u32; 32 * 32];
        let mut p = Painter::new(&mut px, 32, 32, 1.0);
        assert!(icons.draw_file(&mut p, b"song.flac", false, Rect::new(0.0, 0.0, 32.0, 32.0)));
    }

    #[test]
    fn bad_data_is_refused() {
        let mut icons = Icons::new();
        assert!(!icons.add("bad", "M1 2 3"));
        assert!(icons.add("mine", "M2 2 L22 22"));
    }
}
