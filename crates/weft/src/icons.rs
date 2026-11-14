// SPDX-License-Identifier: GPL-3.0-only
//! Weft's icon set: one consistent line style on a 24-unit grid, drawn as vector paths
//! at any scale (1x, 1.25x, 2x look the same, only sharper).
//!
//! Each icon is SVG path data (`M L H V C Q Z`, upper and lower case) plus two
//! shorthands: `O cx cy r` for a circle and `D cx cy r` for a filled dot. Strokes are
//! 2 units with round caps and joins, like most modern sets. A program adds its own
//! icons with [`Icons::add`] in the same notation. These were drawn for SexOS (GPL-3).
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Stroke};

use crate::geom::Rect;
use crate::paint::Painter;
use crate::theme::Color;

/// (name, path data). Grid: 0..24, content kept within 2..22.
pub const BUILTIN: &[(&str, &str)] = &[
    ("menu", "M4 6 H20 M4 12 H20 M4 18 H20"),
    ("close", "M6 6 L18 18 M18 6 L6 18"),
    ("plus", "M12 5 V19 M5 12 H19"),
    ("minus", "M5 12 H19"),
    ("check", "M5 12.5 L10 17.5 L19 7"),
    ("chevron-left", "M15 5 L8 12 L15 19"),
    ("chevron-right", "M9 5 L16 12 L9 19"),
    ("chevron-up", "M5 15 L12 8 L19 15"),
    ("chevron-down", "M5 9 L12 16 L19 9"),
    ("arrow-left", "M19 12 H5 M11 6 L5 12 L11 18"),
    ("arrow-right", "M5 12 H19 M13 6 L19 12 L13 18"),
    ("arrow-up", "M12 19 V5 M6 11 L12 5 L18 11"),
    ("arrow-down", "M12 5 V19 M6 13 L12 19 L18 13"),
    ("search", "O10.5 10.5 6.5 M15.5 15.5 L20 20"),
    ("more", "D5 12 1.6 D12 12 1.6 D19 12 1.6"),
    ("more-vertical", "D12 5 1.6 D12 12 1.6 D12 19 1.6"),
    ("file", "M6 3 H14 L19 8 V21 H6 Z M14 3 V8 H19"),
    ("file-text", "M6 3 H14 L19 8 V21 H6 Z M14 3 V8 H19 M9 13 H16 M9 17 H16"),
    ("folder", "M3 6 Q3 5 4 5 H9 L11 7 H20 Q21 7 21 8 V18 Q21 19 20 19 H4 Q3 19 3 18 Z"),
    ("folder-open", "M3 18 V6 Q3 5 4 5 H9 L11 7 H19 V10 M3 18 L6 11 H22 L19 18 Z"),
    ("save", "M5 3 H16 L21 8 V21 H3 V3 Z M7 3 V8 H15 V3 M7 21 V14 H17 V21"),
    ("copy", "M9 9 H20 V20 H9 Z M5 15 H4 V4 H15 V5"),
    ("cut", "O6 18 3 O18 18 3 M8.2 15.8 L18 4 M15.8 15.8 L6 4"),
    ("paste", "M8 4 H16 V7 H8 Z M8 5.5 H5 V21 H19 V5.5 H16"),
    ("undo", "M9 14 L4 9 L9 4 M4 9 H15 Q20 9 20 14 Q20 19 15 19 H11"),
    ("redo", "M15 14 L20 9 L15 4 M20 9 H9 Q4 9 4 14 Q4 19 9 19 H13"),
    ("trash", "M4 7 H20 M10 3 H14 M6 7 L7 21 H17 L18 7 M10 11 V17 M14 11 V17"),
    ("edit", "M15 4 L20 9 L9 20 H4 V15 Z M13 6 L18 11"),
    ("home", "M3 11 L12 3 L21 11 M5 9.5 V21 H10 V15 H14 V21 H19 V9.5"),
    ("refresh", "M20 6 V11 H15 M19.5 11 Q18 5 12 5 Q5 5 5 12 Q5 19 12 19 Q17 19 19 15"),
    ("info", "O12 12 9 M12 11 V17 D12 7.5 1.2"),
    ("warning", "M12 3 L22 20 H2 Z M12 9 V14 D12 17 1.2"),
    ("error", "O12 12 9 M9 9 L15 15 M15 9 L9 15"),
    ("question", "O12 12 9 M9.5 9.5 Q9.5 7 12 7 Q14.5 7 14.5 9.5 Q14.5 11.5 12 12.5 V14 D12 17 1.2"),
    ("star", "M12 3 L14.7 8.8 L21 9.5 L16.3 13.8 L17.6 20 L12 16.9 L6.4 20 L7.7 13.8 L3 9.5 L9.3 8.8 Z"),
    ("heart", "M12 20 Q3 14 3 8.5 Q3 4.5 7 4.5 Q10 4.5 12 7.5 Q14 4.5 17 4.5 Q21 4.5 21 8.5 Q21 14 12 20 Z"),
    ("play", "M7 4 L19 12 L7 20 Z"),
    ("pause", "M7 4 V20 M17 4 V20"),
    ("stop", "M6 6 H18 V18 H6 Z"),
    ("skip-next", "M5 5 L15 12 L5 19 Z M19 5 V19"),
    ("skip-previous", "M19 5 L9 12 L19 19 Z M5 5 V19"),
    ("volume", "M4 9 H8 L13 5 V19 L8 15 H4 Z M16.5 9 Q18.5 12 16.5 15 M19 6.5 Q23 12 19 17.5"),
    ("image", "M3 5 H21 V19 H3 Z M3 16 L9 10 L14 15 L16.5 12.5 L21 17 D15.5 8.5 1.6"),
    ("music", "M9 18 V5 L20 3 V16 O6.5 18 2.5 O17.5 16 2.5"),
    ("video", "M3 6 H15 V18 H3 Z M15 10 L21 7 V17 L15 14"),
    ("download", "M12 3 V15 M7 10 L12 15 L17 10 M4 19 H20"),
    ("upload", "M12 15 V3 M7 8 L12 3 L17 8 M4 19 H20"),
    ("calculator", "M5 3 H19 V21 H5 Z M8 6 H16 V9 H8 Z D8.5 13 1.1 D12 13 1.1 D15.5 13 1.1 D8.5 17 1.1 D12 17 1.1 D15.5 17 1.1"),
    ("terminal", "M3 5 H21 V19 H3 Z M7 9 L10 12 L7 15 M12 15 H17"),
    ("grid", "M4 4 H10 V10 H4 Z M14 4 H20 V10 H14 Z M4 14 H10 V20 H4 Z M14 14 H20 V20 H14 Z"),
    ("list", "M9 6 H20 M9 12 H20 M9 18 H20 D4.5 6 1.2 D4.5 12 1.2 D4.5 18 1.2"),
    ("settings", "M4 7 H20 M4 17 H20 O9 7 2.5 O15 17 2.5"),
    ("sun", "O12 12 4 M12 2 V4 M12 20 V22 M2 12 H4 M20 12 H22 M4.9 4.9 L6.3 6.3 M17.7 17.7 L19.1 19.1 M4.9 19.1 L6.3 17.7 M17.7 6.3 L19.1 4.9"),
    ("moon", "M20 14.5 Q18 16 15.5 16 Q8 16 8 8.5 Q8 6 9.5 4 Q4 5.5 4 12 Q4 20 12 20 Q18 20 20 14.5 Z"),
    ("user", "O12 8 4 M4 21 Q4 14 12 14 Q20 14 20 21"),
    ("lock", "M5 11 H19 V21 H5 Z M8 11 V7 Q8 3 12 3 Q16 3 16 7 V11"),
    ("globe", "O12 12 9 M3 12 H21 M12 3 Q7 12 12 21 M12 3 Q17 12 12 21"),
    ("bell", "M6 17 V11 Q6 5 12 5 Q18 5 18 11 V17 L20 19 H4 Z M10 21 H14"),
    ("tab", "M3 7 Q3 5 5 5 H10 L12 8 H19 Q21 8 21 10 V18 Q21 19 20 19 H4 Q3 19 3 18 Z"),
    ("split", "M3 4 H21 V20 H3 Z M12 4 V20"),
    ("pin", "M9 3 H15 M10 3 V9 L6 13 H18 L14 9 V3 M12 13 V21"),
    ("link", "M10 14 Q8 16 6 14 L5 13 Q3 11 5 9 L8 6 Q10 4 12 6 M14 10 Q16 8 18 10 L19 11 Q21 13 19 15 L16 18 Q14 20 12 18"),
    ("eye", "M2 12 Q7 5 12 5 Q17 5 22 12 Q17 19 12 19 Q7 19 2 12 Z O12 12 3"),
    ("command", "M9 9 H15 V15 H9 Z M9 9 V6 Q9 3 6 3 Q3 3 3 6 Q3 9 6 9 Z M15 9 V6 Q15 3 18 3 Q21 3 21 6 Q21 9 18 9 Z M9 15 V18 Q9 21 6 21 Q3 21 3 18 Q3 15 6 15 Z M15 15 V18 Q15 21 18 21 Q21 21 21 18 Q21 15 18 15 Z"),
    ("keyboard", "M2 6 H22 V18 H2 Z M6 10 H6.5 M10 10 H10.5 M14 10 H14.5 M18 10 H18.5 M7 14 H17"),
    ("palette", "M12 3 Q21 3 21 11 Q21 15 17 15 H15 Q13 15 13 17 Q13 18 13.5 19 Q14 21 12 21 Q3 21 3 12 Q3 3 12 3 Z D7.5 11 1.2 D10 7 1.2 D15 7 1.2"),
    ("font", "M4 20 L10 4 H12 L18 20 M6.5 14 H15.5"),
];

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
        for (name, data) in BUILTIN { icons.add(name, data); }
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

/// Splits path data into stroked paths and filled dots/circles.
fn parse(data: &str) -> Option<Vec<(Part, Path)>> {
    let mut parts = Vec::new();
    let mut pb = PathBuilder::new();
    let mut nums: Vec<f32> = Vec::new();
    let mut cmd = ' ';
    let (mut x, mut y, mut sx, mut sy) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut tokens = Tokens { s: data.as_bytes(), i: 0 };
    let flush = |cmd: char, nums: &mut Vec<f32>, pb: &mut PathBuilder, parts: &mut Vec<(Part, Path)>, x: &mut f32, y: &mut f32, sx: &mut f32, sy: &mut f32| -> Option<()> {
        let rel = cmd.is_ascii_lowercase();
        let (bx, by) = if rel { (*x, *y) } else { (0.0, 0.0) };
        let n = nums.len();
        let take = |k: usize| -> Option<()> { if n == 0 || n % k != 0 { None } else { Some(()) } };
        match cmd.to_ascii_uppercase() {
            ' ' => {}
            'M' => {
                take(2)?;
                for (i, c) in nums.chunks(2).enumerate() {
                    let (px, py) = (bx + c[0], by + c[1]);
                    if i == 0 { pb.move_to(px, py); *sx = px; *sy = py; } else { pb.line_to(px, py); }
                    *x = px; *y = py;
                }
            }
            'L' => { take(2)?; for c in nums.chunks(2) { let (px, py) = if rel { (*x + c[0], *y + c[1]) } else { (c[0], c[1]) }; pb.line_to(px, py); *x = px; *y = py; } }
            'H' => { take(1)?; for c in nums.iter() { *x = if rel { *x + c } else { *c }; pb.line_to(*x, *y); } }
            'V' => { take(1)?; for c in nums.iter() { *y = if rel { *y + c } else { *c }; pb.line_to(*x, *y); } }
            'Q' => { take(4)?; for c in nums.chunks(4) { let (ox, oy) = if rel { (*x, *y) } else { (0.0, 0.0) }; pb.quad_to(ox + c[0], oy + c[1], ox + c[2], oy + c[3]); *x = ox + c[2]; *y = oy + c[3]; } }
            'C' => { take(6)?; for c in nums.chunks(6) { let (ox, oy) = if rel { (*x, *y) } else { (0.0, 0.0) }; pb.cubic_to(ox + c[0], oy + c[1], ox + c[2], oy + c[3], ox + c[4], oy + c[5]); *x = ox + c[4]; *y = oy + c[5]; } }
            'Z' => { pb.close(); *x = *sx; *y = *sy; }
            'O' | 'D' => {
                take(3)?;
                for c in nums.chunks(3) {
                    let circle = PathBuilder::from_circle(c[0], c[1], c[2])?;
                    parts.push((if cmd.to_ascii_uppercase() == 'O' { Part::Stroke } else { Part::Fill }, circle));
                }
            }
            _ => return None,
        }
        nums.clear();
        Some(())
    };
    while let Some(t) = tokens.next() {
        match t {
            Tok::Cmd(c) => {
                flush(cmd, &mut nums, &mut pb, &mut parts, &mut x, &mut y, &mut sx, &mut sy)?;
                if c.to_ascii_uppercase() == 'Z' { flush(c, &mut nums, &mut pb, &mut parts, &mut x, &mut y, &mut sx, &mut sy)?; cmd = ' '; } else { cmd = c; }
            }
            Tok::Num(v) => nums.push(v),
        }
    }
    flush(cmd, &mut nums, &mut pb, &mut parts, &mut x, &mut y, &mut sx, &mut sy)?;
    if let Some(path) = pb.finish() { parts.insert(0, (Part::Stroke, path)); }
    (!parts.is_empty()).then_some(parts)
}

enum Tok { Cmd(char), Num(f32) }
struct Tokens<'a> { s: &'a [u8], i: usize }
impl Iterator for Tokens<'_> {
    type Item = Tok;
    fn next(&mut self) -> Option<Tok> {
        while self.i < self.s.len() && (self.s[self.i] == b' ' || self.s[self.i] == b',') { self.i += 1; }
        let c = *self.s.get(self.i)?;
        if c.is_ascii_alphabetic() { self.i += 1; return Some(Tok::Cmd(c as char)); }
        let start = self.i;
        self.i += 1;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || self.s[self.i] == b'.') { self.i += 1; }
        core::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok().map(Tok::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_builtin_icon_parses_and_draws_at_1x_and_2x() {
        let icons = Icons::new();
        assert_eq!(icons.names().count(), BUILTIN.len());
        for scale in [1.0f32, 2.0] {
            let n = (24.0 * scale) as usize;
            for (name, _) in BUILTIN {
                let mut px = alloc::vec![0xFFFFFFFFu32; n * n];
                let mut p = Painter::new(&mut px, n, n, scale);
                assert!(icons.draw(&mut p, name, Rect::new(0.0, 0.0, 24.0, 24.0), Color::hex(0)));
                let ink = px.iter().filter(|c| **c & 0xFF < 128).count();
                assert!(ink * 100 / (n * n) >= 2, "{name} at {scale}x has almost no ink ({ink})");
            }
        }
    }
    #[test]
    fn bad_data_is_refused() {
        let mut icons = Icons::new();
        assert!(!icons.add("bad", "M1 2 3"));
        assert!(icons.add("mine", "M2 2 L22 22"));
    }
}
