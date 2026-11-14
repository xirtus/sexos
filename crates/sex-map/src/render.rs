// SPDX-License-Identifier: GPL-3.0-only
//! Drawing: vector tiles (OpenMapTiles and Mapbox Streets layer names), raster tiles,
//! the route line, markers and labels, into a tiny-skia pixmap.
use crate::mvt::{GeomType, Tile};
use crate::View;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use tiny_skia::{Color, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Map colours, 0x00RRGGBB (the gate counts them on screen).
pub const LAND: u32 = 0x00F2_EFE9;
pub const WATER: u32 = 0x00AA_D3DF;
pub const PARK: u32 = 0x00C8_E6A0;
pub const BUILDING: u32 = 0x00D9_D0C9;
pub const BUILDING_EDGE: u32 = 0x00C4_B6AB;
pub const MAJOR_ROAD: u32 = 0x00FC_D6A4;
pub const MOTORWAY: u32 = 0x00E8_92A2;
pub const MINOR_ROAD: u32 = 0x00FF_FFFF;
pub const CASING: u32 = 0x00B8_B0A8;
pub const RAIL: u32 = 0x0099_9999;
pub const ROUTE: u32 = 0x0026_6BE0;
pub const MARKER: u32 = 0x00D3_3A2C;
pub const LABEL: u32 = 0x0033_3333;

fn color(c: u32) -> Color { Color::from_rgba8((c >> 16) as u8, (c >> 8) as u8, c as u8, 255) }
fn paint(c: u32) -> Paint<'static> { let mut p = Paint::default(); p.set_color(color(c)); p.anti_alias = true; p }

/// A label to place once every tile is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Label { pub text: String, pub x: f32, pub y: f32, pub rank: u8 }

/// What a feature is drawn as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind { Park, Water, Building, Road(u8), Rail }

/// Road classes, widest first: (fill, width at 256 px per tile, rank).
fn road_style(class: &str) -> (u32, f32, u8) {
    match class {
        "motorway" | "trunk" => (MOTORWAY, 7.0, 0),
        "primary" => (MAJOR_ROAD, 6.0, 1),
        "secondary" | "tertiary" => (MAJOR_ROAD, 5.0, 2),
        "path" | "track" | "pedestrian" => (MINOR_ROAD, 1.6, 4),
        _ => (MINOR_ROAD, 3.5, 3),
    }
}

fn kind_of(layer: &str, class: Option<&str>, geom: GeomType) -> Option<Kind> {
    Some(match (layer, geom) {
        ("water", GeomType::Polygon) => Kind::Water,
        ("waterway", GeomType::Line) => Kind::Water,
        ("park", GeomType::Polygon) => Kind::Park,
        ("landcover" | "landuse", GeomType::Polygon) if matches!(class, Some("grass" | "wood" | "park" | "forest" | "meadow" | "cemetery" | "pitch")) => Kind::Park,
        ("building", GeomType::Polygon) => Kind::Building,
        ("transportation" | "road", GeomType::Line) if matches!(class, Some("rail" | "transit")) => Kind::Rail,
        ("transportation" | "road", GeomType::Line) => Kind::Road(road_style(class.unwrap_or("minor")).2),
        _ => return None,
    })
}

/// Draws a vector tile whose top left is at (`x`, `y`) and which is `size` pixels
/// wide; returns its labels (place names, street names, water names).
pub fn draw_vector(pm: &mut Pixmap, tile: &Tile, x: f64, y: f64, size: f64) -> Vec<Label> {
    let mut labels = Vec::new();
    let scale = (size / 256.0) as f32;
    // Gather paths per kind, then draw in kind order.
    let mut items: Vec<(Kind, tiny_skia::Path, u32, f32)> = Vec::new();
    for layer in &tile.layers {
        let k = size / layer.extent as f64;
        let at = |p: (i32, i32)| ((x + p.0 as f64 * k) as f32, (y + p.1 as f64 * k) as f32);
        for f in &layer.features {
            let class = f.str("class");
            if let Some(name) = f.str("name").or_else(|| f.str("name:latin")) {
                if f.kind == GeomType::Point || layer.name.ends_with("_name") || layer.name == "place_label" {
                    if let Some(p) = f.parts.first().and_then(|p| p.get(p.len() / 2)) {
                        let (lx, ly) = at(*p);
                        let rank = if layer.name.starts_with("place") { 0 } else if layer.name == "water_name" { 1 } else { 2 };
                        labels.push(Label { text: name.into(), x: lx, y: ly, rank });
                    }
                }
            }
            let Some(kind) = kind_of(&layer.name, class, f.kind) else { continue };
            let mut pb = PathBuilder::new();
            for part in &f.parts {
                let Some(first) = part.first() else { continue };
                let p = at(*first);
                pb.move_to(p.0, p.1);
                for q in &part[1..] { let q = at(*q); pb.line_to(q.0, q.1); }
                if f.kind == GeomType::Polygon { pb.close(); }
            }
            let Some(path) = pb.finish() else { continue };
            let (fill, width) = match kind {
                Kind::Road(_) => { let (c, w, _) = road_style(class.unwrap_or("minor")); (c, w * scale) }
                Kind::Rail => (RAIL, 1.5 * scale),
                Kind::Water if f.kind == GeomType::Line => (WATER, 3.0 * scale),
                Kind::Water => (WATER, 0.0),
                Kind::Park => (PARK, 0.0),
                Kind::Building => (BUILDING, 0.0),
            };
            items.push((kind, path, fill, width));
        }
    }
    items.sort_by(|a, b| a.0.cmp(&b.0));
    // Polygons and waterways.
    for (kind, path, fill, width) in &items {
        match kind {
            Kind::Park | Kind::Water | Kind::Building if *width == 0.0 => {
                pm.fill_path(path, &paint(*fill), FillRule::EvenOdd, Transform::identity(), None);
                if *kind == Kind::Building && scale >= 0.9 {
                    pm.stroke_path(path, &paint(BUILDING_EDGE), &Stroke { width: 1.0, ..Stroke::default() }, Transform::identity(), None);
                }
            }
            Kind::Water => stroke(pm, path, *fill, *width),
            _ => {}
        }
    }
    // Road casings under every road fill, minor roads under major ones.
    let mut roads: Vec<&(Kind, tiny_skia::Path, u32, f32)> = items.iter().filter(|i| matches!(i.0, Kind::Road(_) | Kind::Rail)).collect();
    roads.sort_by(|a, b| b.0.cmp(&a.0));
    for (kind, path, _, width) in &roads { if matches!(kind, Kind::Road(_)) { stroke(pm, path, CASING, width + 2.0 * scale.max(0.5)); } }
    for (_, path, fill, width) in &roads { stroke(pm, path, *fill, *width); }
    labels
}

fn stroke(pm: &mut Pixmap, path: &tiny_skia::Path, c: u32, width: f32) {
    let s = Stroke { width: width.max(0.75), line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
    pm.stroke_path(path, &paint(c), &s, Transform::identity(), None);
}

/// Draws a raster tile (`w` x `h` pixels 0xAARRGGBB) stretched to `size` at (`x`, `y`).
pub fn draw_raster(pm: &mut Pixmap, pixels: &[u32], w: usize, h: usize, x: f64, y: f64, size: f64) {
    let (pw, ph) = (pm.width() as i64, pm.height() as i64);
    let data = pm.data_mut();
    let (x0, y0) = (libm::floor(x) as i64, libm::floor(y) as i64);
    let n = libm::ceil(size) as i64;
    for dy in 0..n {
        let sy = y0 + dy;
        if sy < 0 || sy >= ph { continue; }
        let ty = ((dy as f64 / size) * h as f64) as usize;
        for dx in 0..n {
            let sx = x0 + dx;
            if sx < 0 || sx >= pw { continue; }
            let tx = ((dx as f64 / size) * w as f64) as usize;
            let Some(&p) = pixels.get(ty.min(h - 1) * w + tx.min(w - 1)) else { continue };
            let i = ((sy * pw + sx) * 4) as usize;
            data[i] = (p >> 16) as u8; data[i + 1] = (p >> 8) as u8; data[i + 2] = p as u8; data[i + 3] = 255;
        }
    }
}

/// The route line through `points` (latitude, longitude).
pub fn draw_route(pm: &mut Pixmap, view: &View, points: &[(f64, f64)]) {
    let mut pb = PathBuilder::new();
    for (i, p) in points.iter().enumerate() {
        let (x, y) = view.to_screen(p.0, p.1);
        if i == 0 { pb.move_to(x as f32, y as f32) } else { pb.line_to(x as f32, y as f32) }
    }
    if let Some(path) = pb.finish() {
        stroke(pm, &path, 0x00FF_FFFF, 9.0);
        stroke(pm, &path, ROUTE, 6.0);
    }
}

/// A pin at a place: a filled circle with a white ring.
pub fn draw_marker(pm: &mut Pixmap, view: &View, lat: f64, lon: f64, c: u32) {
    let (x, y) = view.to_screen(lat, lon);
    if let Some(circle) = PathBuilder::from_circle(x as f32, y as f32, 8.0) {
        pm.fill_path(&circle, &paint(0x00FF_FFFF), FillRule::Winding, Transform::identity(), None);
    }
    if let Some(circle) = PathBuilder::from_circle(x as f32, y as f32, 6.0) {
        pm.fill_path(&circle, &paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

/// Label text in Geist Mono cells, haloed so it reads over any colour.
pub struct Text { font: sex_font::Font<'static>, w: usize, h: usize, acc: Vec<i32>, cell: Vec<u8> }

impl Text {
    pub fn new(w: usize, h: usize) -> Option<Text> {
        Some(Text { font: sex_font::Font::new(sex_font::GEIST_MONO)?, w, h, acc: vec![0; sex_font::raster::scratch_len(w, h)], cell: vec![0; w * h] })
    }
    pub fn width(&self, s: &str) -> usize { s.chars().count() * self.w }
    pub fn height(&self) -> usize { self.h }

    /// Draws `s` with its top left at (`x`, `y`); `halo` draws a light ring first.
    pub fn draw(&mut self, pm: &mut Pixmap, s: &str, x: i32, y: i32, c: u32, halo: bool) {
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        let mut at = x;
        for ch in s.chars() {
            self.acc.fill(0);
            self.cell.fill(0);
            if ch != ' ' && sex_font::draw_cell(&self.font, ch as u32, self.w, self.h, &mut self.acc, &mut self.cell) {
                let data = pm.data_mut();
                let passes: &[(i32, i32, u32)] = if halo { &[(-1, 0, 0xFFFFFF), (1, 0, 0xFFFFFF), (0, -1, 0xFFFFFF), (0, 1, 0xFFFFFF), (0, 0, u32::MAX)] } else { &[(0, 0, u32::MAX)] };
                for &(ox, oy, pc) in passes {
                    let col = if pc == u32::MAX { c } else { pc };
                    for gy in 0..self.h as i32 {
                        for gx in 0..self.w as i32 {
                            let a = self.cell[(gy * self.w as i32 + gx) as usize] as u32;
                            let (px, py) = (at + gx + ox, y + gy + oy);
                            if a == 0 || px < 0 || py < 0 || px >= pw || py >= ph { continue; }
                            let i = ((py * pw + px) * 4) as usize;
                            for (k, sh) in [(0usize, 16u32), (1, 8), (2, 0)] {
                                let s = (col >> sh) & 0xFF;
                                data[i + k] = ((s * a + data[i + k] as u32 * (255 - a)) / 255) as u8;
                            }
                        }
                    }
                }
            }
            at += self.w as i32;
        }
    }

    /// Places labels centred on their points, most important first, skipping any
    /// that would overlap one already placed. Returns how many were drawn.
    pub fn place(&mut self, pm: &mut Pixmap, mut labels: Vec<Label>) -> usize {
        labels.sort_by_key(|l| l.rank);
        let mut taken: Vec<(i32, i32, i32, i32)> = Vec::new();
        let mut drawn = 0;
        for l in labels {
            let w = self.width(&l.text) as i32;
            let (x, y) = (l.x as i32 - w / 2, l.y as i32 - self.h as i32 / 2);
            if x + w < 0 || y + self.h as i32 <= 0 || x >= pm.width() as i32 || y >= pm.height() as i32 { continue; }
            let r = (x - 4, y - 2, x + w + 4, y + self.h as i32 + 2);
            if taken.iter().any(|t| r.0 < t.2 && t.0 < r.2 && r.1 < t.3 && t.1 < r.3) { continue; }
            taken.push(r);
            self.draw(pm, &l.text, x, y, LABEL, true);
            drawn += 1;
        }
        drawn
    }
}

/// The pixmap as window pixels 0x00RRGGBB.
pub fn to_window(pm: &Pixmap, out: &mut [u32]) {
    for (o, p) in out.iter_mut().zip(pm.data().chunks_exact(4)) { *o = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32; }
}

/// How many pixels are exactly colour `c` (0x00RRGGBB).
pub fn count(pm: &Pixmap, c: u32) -> usize {
    pm.data().chunks_exact(4).filter(|p| (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32 == c).count()
}

/// A pixmap filled with the land colour.
pub fn blank(w: u32, h: u32) -> Option<Pixmap> {
    let mut pm = Pixmap::new(w.max(1), h.max(1))?;
    pm.fill(color(LAND));
    Some(pm)
}
