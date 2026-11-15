// SPDX-License-Identifier: GPL-3.0-only
//! The drawing: one VectorCraft session holding one document. Everything Gestalt does to the
//! art goes through the engine's commands and tools, so undo, Smart Guides, Pathfinder and the
//! file formats are VectorCraft's own. This module is plain std and runs under `cargo test`.
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_tools::{Mods, Overlay, PointerEvent, PointerKind, ToolKey};

/// The tools Gestalt offers, with the key that picks each (Illustrator's) and its name.
pub const TOOLS: &[(char, &str, &str)] = &[
    ('v', "selection", "Selection"),
    ('a', "directSelection", "Direct Selection"),
    ('p', "pen", "Pen"),
    ('n', "pencil", "Pencil"),
    ('m', "rectangle", "Rectangle"),
    ('l', "ellipse", "Ellipse"),
    ('\\', "lineSegment", "Line Segment"),
    ('t', "type", "Type"),
    ('i', "eyedropper", "Eyedropper"),
    ('h', "hand", "Hand"),
];

pub struct Drawing {
    s: Session,
    /// The file it saves to.
    pub path: String,
    renderer: vectorcraft_render::Renderer,
}

fn err(e: impl std::fmt::Display) -> String { e.to_string() }

/// The format a file name asks for: SVG unless it says otherwise.
fn format_of(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".vectorcraft") { "vectorcraft" } else if lower.ends_with(".pdf") { "pdf" } else if lower.ends_with(".svgz") { "svgz" } else { "svg" }
}

impl Drawing {
    /// A new document, `width` by `height` points, that saves to `path`.
    pub fn blank(path: &str, width: f64, height: f64) -> Drawing {
        let mut s = Session::new();
        s.execute("file.new", &json!({ "width": width, "height": height })).expect("a new document");
        let mut d = Drawing { s, path: path.to_string(), renderer: vectorcraft_render::Renderer::new() };
        d.tool("selection");
        d
    }

    /// Opens `bytes` (SVG, PDF, `.ai`, EPS or `.vectorcraft`) as `path`.
    pub fn open(path: &str, bytes: &[u8]) -> Result<Drawing, String> {
        let mut s = Session::new();
        let name = path.rsplit('/').next().unwrap_or(path);
        s.execute("document.open", &json!({ "name": name, "dataBase64": B64.encode(bytes) })).map_err(err)?;
        let mut d = Drawing { s, path: path.to_string(), renderer: vectorcraft_render::Renderer::new() };
        d.tool("selection");
        Ok(d)
    }

    /// Runs one engine command.
    pub fn command(&mut self, id: &str, params: Value) -> Result<Value, String> { self.s.execute(id, &params).map_err(err) }

    /// Saves to [`Self::path`] in the format its name asks for; returns the bytes written.
    pub fn save(&mut self) -> Result<usize, String> {
        let format = format_of(&self.path);
        self.s.execute("document.save", &json!({ "path": self.path, "format": format, "acknowledgeLoss": true })).map_err(err)?;
        std::fs::metadata(&self.path).map(|m| m.len() as usize).map_err(|e| format!("Could not save {}: {e}", self.path))
    }

    /// The document in `format` (svg, png, pdf, vectorcraft, …).
    pub fn serialize(&mut self, format: &str) -> Result<Vec<u8>, String> {
        let r = self.s.execute("document.serialize", &json!({ "format": format })).map_err(err)?;
        if let Some(text) = r.get("text").and_then(Value::as_str) { return Ok(text.as_bytes().to_vec()); }
        let b64 = r.get("dataBase64").and_then(Value::as_str).ok_or("the engine returned no data")?;
        B64.decode(b64).map_err(err)
    }

    /// Writes the document beside its file as `format` (png or pdf); returns the name and size.
    pub fn export(&mut self, format: &str) -> Result<(String, usize), String> {
        let data = self.serialize(format)?;
        let stem = self.path.rsplit_once('.').map_or(self.path.as_str(), |(s, _)| s);
        let name = format!("{stem}.{format}");
        std::fs::write(&name, &data).map_err(|e| format!("Could not write {name}: {e}"))?;
        Ok((name, data.len()))
    }

    pub fn dirty(&self) -> bool { self.s.doc().is_ok_and(|d| d.is_dirty()) }

    /// The first artboard, in points.
    pub fn artboard(&self) -> Rect {
        self.s.doc().ok().and_then(|d| d.doc.artboards.first().map(|a| a.rect)).unwrap_or(Rect::new(0.0, 0.0, 800.0, 600.0))
    }

    /// The objects at the top of the document (inside its layers).
    pub fn objects(&self) -> usize {
        let Ok(d) = self.s.doc() else { return 0 };
        d.doc.children(None).map_or(0, |layers| layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum())
    }

    pub fn selected(&self) -> usize { self.s.doc().map_or(0, |d| d.selection.objects.len()) }

    /// The bounds of the selection, in points.
    pub fn selection_bounds(&self) -> Option<Rect> {
        let d = self.s.doc().ok()?;
        if d.selection.objects.is_empty() { return None; }
        d.doc.bounds_of(&d.selection.objects, false)
    }

    pub fn view(zoom: f64) -> ViewInfo { ViewInfo { zoom, smart_guides: true, ..ViewInfo::default() } }

    /// Picks a tool by its engine id.
    pub fn tool(&mut self, id: &str) -> bool { self.s.select_tool(id, Self::view(1.0)).is_ok() }

    pub fn tool_id(&self) -> &'static str { self.s.tool_id() }

    /// One pointer event at `at` (points) with `mods` held.
    pub fn pointer(&mut self, kind: PointerKind, at: Point, mods: Mods, zoom: f64) -> Result<(), String> {
        self.s.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(mods), Self::view(zoom)).map(|_| ()).map_err(err)
    }

    /// A key for the active tool (Enter, Escape, arrows nudge, Delete…).
    pub fn key(&mut self, key: ToolKey, mods: Mods, zoom: f64) -> Result<(), String> {
        self.s.tool_key(key, mods, Self::view(zoom)).map(|_| ()).map_err(err)
    }

    /// Text typed into the active tool (the Type tool's frame).
    pub fn text(&mut self, text: &str, zoom: f64) -> Result<(), String> {
        self.s.tool_text(text, Self::view(zoom)).map(|_| ()).map_err(err)
    }

    pub fn wants_text(&self) -> bool { self.s.tool_wants_text() }

    pub fn overlays(&mut self, zoom: f64) -> Vec<Overlay> { self.s.overlays(Self::view(zoom)) }

    pub fn overlays_len(&mut self, zoom: f64) -> usize { self.overlays(zoom).len() }

    /// Moves on every change to the document or its selection.
    pub fn revision(&self) -> u64 { self.s.doc().map_or(0, |d| d.revision) }

    /// The document drawn `width` by `height` pixels, `zoom` pixels a point, with the point
    /// `origin` at the top left; 0x00RRGGBB over `desk` outside the art.
    pub fn render(&mut self, width: u32, height: u32, zoom: f64, origin: Point, desk: u32) -> Vec<u32> {
        let Ok(d) = self.s.doc() else { return vec![desk; (width * height) as usize] };
        let doc = d.doc.clone();
        let view = Affine::scale(zoom) * Affine::translate((-origin.x, -origin.y));
        let opts = vectorcraft_render::RenderOptions { artboards: true, ..Default::default() };
        let out = self.renderer.render(&doc, width, height, view, &opts);
        let (dr, dg, db) = ((desk >> 16) & 0xff, (desk >> 8) & 0xff, desk & 0xff);
        out.pixels
            .chunks_exact(4)
            .map(|p| {
                // Premultiplied RGBA over the desk.
                let a = 255 - p[3] as u32;
                let r = p[0] as u32 + dr * a / 255;
                let g = p[1] as u32 + dg * a / 255;
                let b = p[2] as u32 + db * a / 255;
                (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { shift: false, alt: false, cmd: false, ctrl: false, space: false };

    fn drag(d: &mut Drawing, a: (f64, f64), b: (f64, f64)) {
        d.pointer(PointerKind::Down, Point::new(a.0, a.1), NONE, 1.0).unwrap();
        d.pointer(PointerKind::Drag, Point::new((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0), NONE, 1.0).unwrap();
        d.pointer(PointerKind::Drag, Point::new(b.0, b.1), NONE, 1.0).unwrap();
        d.pointer(PointerKind::Up, Point::new(b.0, b.1), NONE, 1.0).unwrap();
    }

    #[test]
    fn draws_moves_undoes_and_saves_svg() {
        let dir = std::env::temp_dir().join(format!("gestalt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.svg").to_string_lossy().into_owned();
        let mut d = Drawing::blank(&path, 400.0, 300.0);
        assert_eq!(d.artboard().width(), 400.0);
        assert!(!d.dirty());
        assert!(d.tool("rectangle"));
        drag(&mut d, (100.0, 100.0), (200.0, 180.0));
        assert_eq!(d.objects(), 1);
        let b = d.selection_bounds().expect("the new rectangle is selected");
        assert_eq!((b.width(), b.height()), (100.0, 80.0));
        assert!(d.dirty());
        assert!(d.tool("ellipse"));
        drag(&mut d, (250.0, 50.0), (350.0, 150.0));
        assert_eq!(d.objects(), 2);
        // Selection: a drag on the ellipse moves it.
        assert!(d.tool("selection"));
        drag(&mut d, (300.0, 100.0), (320.0, 130.0));
        let b = d.selection_bounds().unwrap();
        assert_eq!((b.x0, b.y0), (270.0, 80.0));
        d.command("edit.undo", json!({})).unwrap();
        assert_eq!(d.selection_bounds().map(|b| (b.x0, b.y0)), Some((250.0, 50.0)));
        // Draws: some pixels are not paper.
        let px = d.render(400, 300, 1.0, Point::new(0.0, 0.0), 0x00808080);
        assert_eq!(px.len(), 400 * 300);
        assert_eq!(px[0], 0x00ffffff, "the artboard is white");
        assert!(px.iter().any(|p| *p != 0x00ffffff), "the art shows");
        let n = d.save().unwrap();
        assert!(!d.dirty());
        let svg = std::fs::read_to_string(&path).unwrap();
        assert_eq!(svg.len(), n);
        assert!(svg.contains("<svg") && svg.contains("<rect") || svg.contains("<path"), "{svg}");
        // It opens again with both objects.
        let again = Drawing::open(&path, svg.as_bytes()).unwrap();
        assert_eq!(again.objects(), 2);
        let (png, bytes) = d.export("png").unwrap();
        assert!(png.ends_with("a.png") && bytes > 100);
        assert_eq!(&std::fs::read(&png).unwrap()[1..4], b"PNG");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_tool_on_the_strip_exists() {
        let mut d = Drawing::blank("t.svg", 200.0, 200.0);
        for (_, id, _) in TOOLS { assert!(d.tool(id), "{id}"); assert_eq!(d.tool_id(), *id); }
        assert!(d.tool("shapeBuilder"));
    }

    #[test]
    fn pathfinder_unites_two_overlapping_shapes() {
        let mut d = Drawing::blank("p.svg", 400.0, 400.0);
        d.tool("rectangle");
        drag(&mut d, (50.0, 50.0), (150.0, 150.0));
        drag(&mut d, (120.0, 120.0), (220.0, 220.0));
        assert_eq!(d.objects(), 2);
        d.command("select.all", json!({})).unwrap();
        assert_eq!(d.selected(), 2);
        d.command("object.pathfinder.unite", json!({})).unwrap();
        assert_eq!(d.objects(), 1);
        let b = d.selection_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (50.0, 50.0, 220.0, 220.0));
        d.command("edit.clear", json!({})).unwrap();
        assert_eq!(d.objects(), 0);
    }
}
