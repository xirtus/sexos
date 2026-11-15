// SPDX-License-Identifier: GPL-3.0-only
//! Workspaces and their areas, laid out like Blender's default startup file.
//!
//! A workspace is a tree of splits whose leaves are areas; each area shows one
//! editor. Borders between areas can be dragged, and an area's editor type can
//! be changed from the dropdown at the left of its header, as in Blender.
use crate::draw::Rect;
use crate::editors::{Editor, EditorKind};
use crate::theme as t;

pub enum Node {
    /// `horizontal`: children side by side (a | b); otherwise stacked (a over b).
    /// `ratio` is a's share.
    Split { horizontal: bool, ratio: f32, a: Box<Node>, b: Box<Node> },
    Area(Area),
}

pub struct Area {
    pub id: u32,
    pub editor: Editor,
    /// Ctrl+Space toggles a maximised area.
    pub rect: Rect,
}

pub struct Workspace {
    pub name: String,
    pub root: Node,
    /// Area id shown full-screen (Ctrl+Space), if any.
    pub maximized: Option<u32>,
}

/// A draggable border: the split's path from the root (false = a, true = b).
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    pub path: Vec<bool>,
    pub rect: Rect,
    pub horizontal: bool,
    /// The split's whole rectangle, to turn a pointer position into a ratio.
    pub parent: Rect,
}

fn split(horizontal: bool, ratio: f32, a: Node, b: Node) -> Node {
    Node::Split { horizontal, ratio, a: Box::new(a), b: Box::new(b) }
}

impl Workspace {
    /// Assign rectangles to every area inside `r`; returns the borders.
    pub fn layout(&mut self, r: Rect) -> Vec<Border> {
        let mut borders = Vec::new();
        if let Some(max) = self.maximized {
            if let Some(a) = self.area_mut(max) {
                a.rect = r;
                return borders;
            }
            self.maximized = None;
        }
        layout(&mut self.root, r, &mut Vec::new(), &mut borders);
        borders
    }

    pub fn areas(&self) -> Vec<&Area> {
        let mut out = Vec::new();
        walk(&self.root, &mut |a| out.push(a));
        if let Some(m) = self.maximized {
            out.retain(|a| a.id == m);
        }
        out
    }
    pub fn areas_mut(&mut self) -> Vec<&mut Area> {
        let mut out = Vec::new();
        walk_mut(&mut self.root, &mut out);
        if let Some(m) = self.maximized {
            out.retain(|a| a.id == m);
        }
        out
    }
    pub fn area_mut(&mut self, id: u32) -> Option<&mut Area> {
        let mut all = Vec::new();
        walk_mut(&mut self.root, &mut all);
        all.into_iter().find(|a| a.id == id)
    }
    pub fn area_at(&self, x: i32, y: i32) -> Option<u32> {
        self.areas().into_iter().find(|a| a.rect.contains(x, y)).map(|a| a.id)
    }
    /// Set a split's ratio from a dragged border position.
    pub fn drag_border(&mut self, b: &Border, x: i32, y: i32) {
        let mut node = &mut self.root;
        for &step in &b.path {
            node = match node {
                Node::Split { a, b: bb, .. } => {
                    if step {
                        bb
                    } else {
                        a
                    }
                }
                Node::Area(_) => return,
            };
        }
        if let Node::Split { horizontal, ratio, .. } = node {
            let (pos, len) = if *horizontal { (x - b.parent.x, b.parent.w) } else { (y - b.parent.y, b.parent.h) };
            if len > 0 {
                // Keep at least a header's height (or 60 px) on each side.
                let min = (t::HEADER_H + 4).max(30) as f32 / len as f32;
                *ratio = (pos as f32 / len as f32).clamp(min, 1.0 - min);
            }
        }
    }
    /// Split an area in two (Blender's "Vertical/Horizontal Split").
    pub fn split_area(&mut self, id: u32, horizontal: bool, new_id: u32) -> bool {
        fn go(n: &mut Node, id: u32, horizontal: bool, new_id: u32) -> bool {
            match n {
                Node::Area(a) if a.id == id => {
                    let kind = a.editor.kind();
                    let old = core::mem::replace(n, Node::Area(Area { id: 0, editor: Editor::new(EditorKind::Info), rect: Rect::default() }));
                    let new = Node::Area(Area { id: new_id, editor: Editor::new(kind), rect: Rect::default() });
                    *n = split(horizontal, 0.5, old, new);
                    true
                }
                Node::Area(_) => false,
                Node::Split { a, b, .. } => go(a, id, horizontal, new_id) || go(b, id, horizontal, new_id),
            }
        }
        go(&mut self.root, id, horizontal, new_id)
    }
    /// Remove an area, its sibling takes the space ("Close Area").
    pub fn close_area(&mut self, id: u32) -> bool {
        fn go(n: &mut Node, id: u32) -> bool {
            if let Node::Split { a, b, .. } = n {
                let a_is = matches!(&**a, Node::Area(x) if x.id == id);
                let b_is = matches!(&**b, Node::Area(x) if x.id == id);
                if a_is || b_is {
                    let keep = if a_is { core::mem::replace(&mut **b, dummy()) } else { core::mem::replace(&mut **a, dummy()) };
                    *n = keep;
                    return true;
                }
                return go(a, id) || go(b, id);
            }
            false
        }
        fn dummy() -> Node {
            Node::Area(Area { id: 0, editor: Editor::new(EditorKind::Info), rect: Rect::default() })
        }
        go(&mut self.root, id)
    }
    pub fn max_id(&self) -> u32 {
        let mut m = 0;
        walk(&self.root, &mut |a| m = m.max(a.id));
        m
    }
}

fn walk<'a>(n: &'a Node, f: &mut dyn FnMut(&'a Area)) {
    match n {
        Node::Area(a) => f(a),
        Node::Split { a, b, .. } => {
            walk(a, f);
            walk(b, f);
        }
    }
}
fn walk_mut<'a>(n: &'a mut Node, out: &mut Vec<&'a mut Area>) {
    match n {
        Node::Area(a) => out.push(a),
        Node::Split { a, b, .. } => {
            walk_mut(a, out);
            walk_mut(b, out);
        }
    }
}

fn layout(n: &mut Node, r: Rect, path: &mut Vec<bool>, borders: &mut Vec<Border>) {
    match n {
        Node::Area(a) => a.rect = r,
        Node::Split { horizontal, ratio, a, b } => {
            let g = t::GAP;
            let (ra, rb, br) = if *horizontal {
                let wa = ((r.w - g) as f32 * *ratio).round() as i32;
                (Rect::new(r.x, r.y, wa, r.h), Rect::new(r.x + wa + g, r.y, r.w - wa - g, r.h), Rect::new(r.x + wa - 2, r.y, g + 4, r.h))
            } else {
                let ha = ((r.h - g) as f32 * *ratio).round() as i32;
                (Rect::new(r.x, r.y, r.w, ha), Rect::new(r.x, r.y + ha + g, r.w, r.h - ha - g), Rect::new(r.x, r.y + ha - 2, r.w, g + 4))
            };
            borders.push(Border { path: path.clone(), rect: br, horizontal: *horizontal, parent: r });
            path.push(false);
            layout(a, ra, path, borders);
            path.pop();
            path.push(true);
            layout(b, rb, path, borders);
            path.pop();
        }
    }
}

/// Blender's default workspaces, in tab order.
pub const WORKSPACES: [&str; 11] =
    ["Layout", "Modeling", "Sculpting", "UV Editing", "Texture Paint", "Shading", "Animation", "Rendering", "Compositing", "Geometry Nodes", "Scripting"];

/// Build the default workspaces with their area layouts (proportions measured
/// from Blender's factory startup at 1920x1080).
pub fn default_workspaces() -> Vec<Workspace> {
    let mut next = 0u32;
    let mut area = |kind: EditorKind| {
        next += 1;
        Node::Area(Area { id: next, editor: Editor::new(kind), rect: Rect::default() })
    };
    use EditorKind as K;
    let mut out = Vec::new();
    // The outliner-over-properties column every workspace has on the right.
    let side = |area: &mut dyn FnMut(EditorKind) -> Node| split(false, 0.33, area(K::Outliner), area(K::Properties));
    // Layout: 3D Viewport over Timeline | Outliner over Properties.
    let l = split(true, 0.78, split(false, 0.83, area(K::View3D), area(K::Timeline)), side(&mut area));
    out.push(Workspace { name: "Layout".into(), root: l, maximized: None });
    let l = split(true, 0.78, area(K::View3D), side(&mut area));
    out.push(Workspace { name: "Modeling".into(), root: l, maximized: None });
    let l = split(true, 0.78, area(K::View3D), side(&mut area));
    out.push(Workspace { name: "Sculpting".into(), root: l, maximized: None });
    let l = split(true, 0.78, split(true, 0.5, area(K::UvEditor), area(K::View3D)), side(&mut area));
    out.push(Workspace { name: "UV Editing".into(), root: l, maximized: None });
    let l = split(true, 0.78, split(true, 0.5, area(K::ImageEditor), area(K::View3D)), side(&mut area));
    out.push(Workspace { name: "Texture Paint".into(), root: l, maximized: None });
    let l = split(
        true,
        0.78,
        split(true, 0.2, split(false, 0.5, area(K::FileBrowser), area(K::ImageEditor)), split(false, 0.55, area(K::View3D), area(K::ShaderEditor))),
        side(&mut area),
    );
    out.push(Workspace { name: "Shading".into(), root: l, maximized: None });
    let l = split(true, 0.78, split(false, 0.7, split(true, 0.25, area(K::View3D), area(K::View3D)), area(K::DopeSheet)), side(&mut area));
    let mut anim = Workspace { name: "Animation".into(), root: l, maximized: None };
    // The small view on the left looks through the camera, as in Blender.
    if let Some(crate::editors::Editor::View3D(v)) = anim.areas_mut().into_iter().next().map(|a| &mut a.editor) {
        v.camera_view = true;
    }
    out.push(anim);
    let l = split(true, 0.78, area(K::ImageEditor), side(&mut area));
    out.push(Workspace { name: "Rendering".into(), root: l, maximized: None });
    let l = split(true, 0.78, split(false, 0.8, area(K::CompositorEditor), area(K::DopeSheet)), side(&mut area));
    out.push(Workspace { name: "Compositing".into(), root: l, maximized: None });
    let l = split(true, 0.78, split(false, 0.5, split(true, 0.45, area(K::Spreadsheet), area(K::View3D)), area(K::GeometryNodeEditor)), side(&mut area));
    out.push(Workspace { name: "Geometry Nodes".into(), root: l, maximized: None });
    let l = split(
        true,
        0.78,
        split(true, 0.3, split(false, 0.4, area(K::View3D), split(false, 0.6, area(K::PythonConsole), area(K::Info))), area(K::TextEditor)),
        side(&mut area),
    );
    out.push(Workspace { name: "Scripting".into(), root: l, maximized: None });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_workspace_matches_blender_startup() {
        let mut ws = default_workspaces();
        assert_eq!(ws.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), WORKSPACES);
        let layout = &mut ws[0];
        let borders = layout.layout(Rect::new(0, 26, 1920, 1032));
        assert_eq!(borders.len(), 3);
        let kinds: Vec<_> = layout.areas().iter().map(|a| (a.editor.kind(), a.rect)).collect();
        // Viewport big on the left, timeline under it, outliner over properties right.
        assert_eq!(kinds[0].0, EditorKind::View3D);
        assert_eq!(kinds[1].0, EditorKind::Timeline);
        assert_eq!(kinds[2].0, EditorKind::Outliner);
        assert_eq!(kinds[3].0, EditorKind::Properties);
        assert!(kinds[0].1.w > 1400 && kinds[0].1.h > 800);
        assert!(kinds[2].1.x > 1450 && kinds[3].1.y > kinds[2].1.bottom());
        // Dragging the main vertical border moves it.
        let b = borders[0].clone();
        layout.drag_border(&b, 1000, 500);
        layout.layout(Rect::new(0, 26, 1920, 1032));
        assert!((layout.areas()[2].rect.x - 1002).abs() <= 2);
    }

    #[test]
    fn split_and_close_area() {
        let mut ws = default_workspaces().remove(1);
        let id = ws.areas()[0].id;
        let n = ws.max_id() + 100;
        assert!(ws.split_area(id, true, n));
        ws.layout(Rect::new(0, 0, 800, 600));
        assert_eq!(ws.areas().len(), 4);
        assert!(ws.close_area(n));
        assert_eq!(ws.areas().len(), 3);
    }
}
