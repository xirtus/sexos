// SPDX-License-Identifier: GPL-3.0-only
//! Node geometry from the tree's data, as Blender lays nodes out: header,
//! then outputs, then the node's buttons (properties), then inputs, then
//! collapsible panels with their inputs. Node space is Blender's (y up, a
//! node's location is its top-left corner); [`View`] maps it to the screen.
use crate::draw::Rect;
use sex_3d::nodes::{edit, update, Node, NodeTree, Prop, SocketType, Subtype, Value};
use std::collections::HashSet;

pub const HEAD: f32 = 24.0;
pub const ROW: f32 = 22.0;
pub const PAD: f32 = 5.0;
pub const RAMP_H: f32 = 3.0 * ROW + 22.0;
pub const CURVE_H: f32 = ROW + 150.0;
pub const REROUTE_R: f32 = 6.0;
pub const FRAME_MARGIN: f32 = 14.0;
pub const FRAME_HEAD: f32 = 30.0;

/// The canvas transform: node-space point at the region's centre and zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub center: (f32, f32),
    pub zoom: f32,
    pub region: Rect,
}

impl View {
    pub fn to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        let (cx, cy) = (self.region.x as f32 + self.region.w as f32 / 2.0, self.region.y as f32 + self.region.h as f32 / 2.0);
        (cx + (p.0 - self.center.0) * self.zoom, cy - (p.1 - self.center.1) * self.zoom)
    }
    pub fn to_node(&self, x: i32, y: i32) -> (f32, f32) {
        let (cx, cy) = (self.region.x as f32 + self.region.w as f32 / 2.0, self.region.y as f32 + self.region.h as f32 / 2.0);
        (self.center.0 + (x as f32 - cx) / self.zoom, self.center.1 - (y as f32 - cy) / self.zoom)
    }
    /// A node-space box (top-left, size) on screen.
    pub fn rect(&self, loc: [f32; 2], w: f32, h: f32) -> Rect {
        let (x, y) = self.to_screen((loc[0], loc[1]));
        Rect::new(x.round() as i32, y.round() as i32, (w * self.zoom).round().max(1.0) as i32, (h * self.zoom).round().max(1.0) as i32)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Output(usize),
    /// A node property (dropdown, checkbox, ramp, curve, data-block).
    Prop(usize),
    /// The output value of a Value or RGB node.
    OutputValue(usize),
    /// An input socket: its label and value field (or label only).
    Input(usize),
    /// Component `k` (X/Y/Z) of an unlinked vector input.
    VectorComp(usize, usize),
    /// A collapsible panel's header.
    Panel(String),
}

#[derive(Clone, Debug)]
pub struct NodeLayout {
    /// Node-space height.
    pub height: f32,
    pub rect: Rect,
    pub rows: Vec<(Rect, RowKind)>,
    /// Socket centres on screen (None when not drawn).
    pub inputs: Vec<Option<(f32, f32)>>,
    pub outputs: Vec<Option<(f32, f32)>>,
}

/// Does an unlinked input of this type have a value field?
pub fn has_value(ty: SocketType) -> bool {
    !matches!(ty, SocketType::Shader | SocketType::Geometry | SocketType::Virtual | SocketType::Matrix | SocketType::Menu)
}

pub fn socket_visible(t: &NodeTree, n: usize, i: usize, input: bool) -> bool {
    let node = &t.nodes[n];
    let s = if input { &node.inputs[i] } else { &node.outputs[i] };
    if !s.enabled {
        return false;
    }
    if s.hide {
        // Hidden sockets still show while something is linked to them.
        return t.links.iter().any(|l| if input { l.to_node == n && l.to_socket == i } else { l.from_node == n && l.from_socket == i });
    }
    true
}

pub fn is_linked(t: &NodeTree, n: usize, i: usize) -> bool {
    t.links.iter().any(|l| l.to_node == n && l.to_socket == i)
}

fn prop_height(p: &Prop) -> f32 {
    match p {
        Prop::Ramp(_) => RAMP_H,
        Prop::Curve(_) => CURVE_H,
        Prop::Vector(_) => 3.0 * ROW,
        _ => ROW,
    }
}

/// Rows of a node in node space: (y offset from the top, height, kind).
pub fn rows(t: &NodeTree, n: usize, open: &HashSet<(String, String)>) -> Vec<(f32, f32, RowKind)> {
    let node = &t.nodes[n];
    let mut out = Vec::new();
    let mut y = HEAD + PAD;
    let mut push = |kind: RowKind, h: f32, out: &mut Vec<(f32, f32, RowKind)>| {
        out.push((y, h, kind));
        y += h;
    };
    for i in 0..node.outputs.len() {
        if socket_visible(t, n, i, false) {
            push(RowKind::Output(i), ROW, &mut out);
        }
    }
    if matches!(node.idname.as_str(), "ShaderNodeValue" | "CompositorNodeValue" | "ShaderNodeRGB" | "CompositorNodeRGB") && !node.outputs.is_empty() {
        let h = if node.outputs[0].ty == SocketType::Color { 2.0 * ROW } else { ROW };
        push(RowKind::OutputValue(0), h, &mut out);
    }
    for (k, (name, p)) in node.props.iter().enumerate() {
        if !update::prop_visible(node, name) {
            continue;
        }
        push(RowKind::Prop(k), prop_height(p), &mut out);
    }
    let input_rows = |i: usize| -> Vec<(f32, RowKind)> {
        let s = &node.inputs[i];
        let mut v = vec![(ROW, RowKind::Input(i))];
        if s.ty == SocketType::Vector && !s.hide_value && !is_linked(t, n, i) && matches!(s.value, Value::Vector(_)) {
            for k in 0..3 {
                v.push((ROW, RowKind::VectorComp(i, k)));
            }
        }
        v
    };
    let mut panels: Vec<String> = Vec::new();
    for i in 0..node.inputs.len() {
        if !socket_visible(t, n, i, true) {
            continue;
        }
        match &node.inputs[i].panel {
            None => {
                for (h, k) in input_rows(i) {
                    push(k, h, &mut out);
                }
            }
            Some(p) => {
                if !panels.contains(p) {
                    panels.push(p.clone());
                }
            }
        }
    }
    for p in panels {
        push(RowKind::Panel(p.clone()), ROW, &mut out);
        if open.contains(&(node.name.clone(), p.clone())) {
            for i in 0..node.inputs.len() {
                if node.inputs[i].panel.as_deref() == Some(p.as_str()) && socket_visible(t, n, i, true) {
                    for (h, k) in input_rows(i) {
                        push(k, h, &mut out);
                    }
                }
            }
        }
    }
    out
}

/// A node's node-space height.
pub fn node_height(t: &NodeTree, n: usize, open: &HashSet<(String, String)>) -> f32 {
    let node = &t.nodes[n];
    match node.idname.as_str() {
        edit::REROUTE => return 2.0 * REROUTE_R,
        edit::FRAME => {
            return match node.prop("height") {
                Some(Prop::Float(h)) => *h,
                _ => 100.0,
            }
        }
        _ => {}
    }
    if node.hide {
        return HEAD;
    }
    let r = rows(t, n, open);
    r.last().map_or(HEAD + PAD, |l| l.0 + l.1) + PAD
}

/// Frames: node-space (left, top, right, bottom) of each frame, grown around
/// the nodes inside it when it shrinks to fit.
pub fn frame_bounds(t: &NodeTree, heights: &[f32]) -> Vec<Option<[f32; 4]>> {
    let n = t.nodes.len();
    let mut out: Vec<Option<[f32; 4]>> = vec![None; n];
    // Innermost frames first: depth order.
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(p) = t.nodes[i].parent {
            d += 1;
            i = p;
            if d > n {
                break;
            }
        }
        d
    };
    let mut frames: Vec<usize> = (0..n).filter(|&i| t.nodes[i].idname == edit::FRAME).collect();
    frames.sort_by_key(|&f| std::cmp::Reverse(depth(f)));
    for f in frames {
        let node = &t.nodes[f];
        let shrink = !matches!(node.prop("shrink"), Some(Prop::Bool(false)));
        let mut b: Option<[f32; 4]> = None;
        if shrink {
            for (i, c) in t.nodes.iter().enumerate() {
                if c.parent != Some(f) {
                    continue;
                }
                let cb = if c.idname == edit::FRAME { out[i] } else { Some([c.location[0], c.location[1], c.location[0] + c.width, c.location[1] - heights[i]]) };
                if let Some(cb) = cb {
                    b = Some(match b {
                        None => cb,
                        Some(o) => [o[0].min(cb[0]), o[1].max(cb[1]), o[2].max(cb[2]), o[3].min(cb[3])],
                    });
                }
            }
        }
        out[f] = Some(match b {
            Some(b) => [b[0] - FRAME_MARGIN, b[1] + FRAME_HEAD, b[2] + FRAME_MARGIN, b[3] - FRAME_MARGIN],
            None => [node.location[0], node.location[1], node.location[0] + node.width, node.location[1] - heights[f]],
        });
    }
    out
}

/// Lay out every node on screen.
pub fn layout(t: &NodeTree, v: &View, open: &HashSet<(String, String)>) -> Vec<NodeLayout> {
    let heights: Vec<f32> = (0..t.nodes.len()).map(|i| node_height(t, i, open)).collect();
    let frames = frame_bounds(t, &heights);
    let z = v.zoom;
    (0..t.nodes.len())
        .map(|n| {
            let node = &t.nodes[n];
            let mut inputs = vec![None; node.inputs.len()];
            let mut outputs = vec![None; node.outputs.len()];
            if node.idname == edit::FRAME {
                let b = frames[n].unwrap_or([node.location[0], node.location[1], node.location[0] + node.width, node.location[1] - heights[n]]);
                let rect = v.rect([b[0], b[1]], b[2] - b[0], b[1] - b[3]);
                return NodeLayout { height: b[1] - b[3], rect, rows: Vec::new(), inputs, outputs };
            }
            if node.idname == edit::REROUTE {
                let rect = v.rect(node.location, 2.0 * REROUTE_R, 2.0 * REROUTE_R);
                let c = (rect.x as f32 + rect.w as f32 / 2.0, rect.y as f32 + rect.h as f32 / 2.0);
                for s in inputs.iter_mut() {
                    *s = Some(c);
                }
                for s in outputs.iter_mut() {
                    *s = Some(c);
                }
                return NodeLayout { height: heights[n], rect, rows: Vec::new(), inputs, outputs };
            }
            let rect = v.rect(node.location, node.width, heights[n]);
            let (left, right) = (rect.x as f32, rect.right() as f32);
            if node.hide {
                // Collapsed: sockets bunched along the rounded ends.
                let vis_in: Vec<usize> = (0..node.inputs.len()).filter(|&i| socket_visible(t, n, i, true)).collect();
                let vis_out: Vec<usize> = (0..node.outputs.len()).filter(|&i| socket_visible(t, n, i, false)).collect();
                let cy = rect.y as f32 + rect.h as f32 / 2.0;
                let step = (8.0 * z).min(rect.h as f32 / (vis_in.len().max(vis_out.len()).max(1)) as f32);
                for (k, &i) in vis_in.iter().enumerate() {
                    inputs[i] = Some((left, cy + (k as f32 - (vis_in.len() as f32 - 1.0) / 2.0) * step));
                }
                for (k, &i) in vis_out.iter().enumerate() {
                    outputs[i] = Some((right, cy + (k as f32 - (vis_out.len() as f32 - 1.0) / 2.0) * step));
                }
                return NodeLayout { height: heights[n], rect, rows: Vec::new(), inputs, outputs };
            }
            let mut rows_out = Vec::new();
            let mut panel_y: Vec<(String, f32)> = Vec::new();
            for (y, h, kind) in rows(t, n, open) {
                let sy = rect.y as f32 + y * z;
                let rr = Rect::new(rect.x, sy.round() as i32, rect.w, (h * z).round().max(1.0) as i32);
                let cy = sy + ROW * z / 2.0;
                match &kind {
                    RowKind::Output(i) => outputs[*i] = Some((right, cy)),
                    RowKind::Input(i) => inputs[*i] = Some((left, cy)),
                    RowKind::Panel(p) => panel_y.push((p.clone(), cy)),
                    _ => {}
                }
                rows_out.push((rr, kind));
            }
            // Linked inputs inside a closed panel sit on its header.
            for (i, s) in node.inputs.iter().enumerate() {
                if inputs[i].is_none() && socket_visible(t, n, i, true) && is_linked(t, n, i) {
                    if let Some(p) = &s.panel {
                        if let Some((_, y)) = panel_y.iter().find(|x| &x.0 == p) {
                            inputs[i] = Some((left, *y));
                        }
                    }
                }
            }
            NodeLayout { height: heights[n], rect, rows: rows_out, inputs, outputs }
        })
        .collect()
}

/// The UI label of an input or output for drawing ("Base Color").
pub fn socket_label(s: &sex_3d::nodes::Socket) -> &str {
    &s.name
}

/// Float field options from a socket's subtype and range.
pub fn float_range(s: &sex_3d::nodes::Socket) -> (f32, f32, bool) {
    let factor = matches!(s.subtype, Subtype::Factor | Subtype::Percentage) && s.min.is_finite() && s.max.is_finite() && s.max > s.min && s.max - s.min <= 1000.0;
    let min = if s.min <= f32::MIN { f32::NEG_INFINITY } else { s.min };
    let max = if s.max >= f32::MAX { f32::INFINITY } else { s.max };
    (min, max, factor)
}

/// A point on a link's curve; Blender's links leave and enter sockets
/// horizontally.
pub fn link_points(a: (f32, f32), b: (f32, f32), zoom: f32, n: usize) -> Vec<(f32, f32)> {
    let dx = ((b.0 - a.0).abs() * 0.5).max(30.0 * zoom);
    let (c1, c2) = ((a.0 + dx, a.1), (b.0 - dx, b.1));
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (u * u * u * a.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * b.0, u * u * u * a.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * b.1)
        })
        .collect()
}

/// Where segments p-q and r-s cross, if they do.
pub fn seg_cross(p: (f32, f32), q: (f32, f32), r: (f32, f32), s: (f32, f32)) -> Option<(f32, f32)> {
    let d = (q.0 - p.0) * (s.1 - r.1) - (q.1 - p.1) * (s.0 - r.0);
    if d.abs() < 1e-9 {
        return None;
    }
    let t = ((r.0 - p.0) * (s.1 - r.1) - (r.1 - p.1) * (s.0 - r.0)) / d;
    let u = ((r.0 - p.0) * (q.1 - p.1) - (r.1 - p.1) * (q.0 - p.0)) / d;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)))
}

/// The node a point is over (frontmost first; frames last).
pub fn node_at(t: &NodeTree, lay: &[NodeLayout], x: i32, y: i32) -> Option<usize> {
    let hit = |i: usize| {
        let r = lay[i].rect;
        if t.nodes[i].idname == edit::REROUTE {
            let (cx, cy) = r.center();
            return (x - cx).pow(2) + (y - cy).pow(2) <= (r.w / 2 + 3).pow(2);
        }
        r.contains(x, y)
    };
    (0..t.nodes.len()).rev().find(|&i| t.nodes[i].idname != edit::FRAME && hit(i)).or_else(|| (0..t.nodes.len()).rev().find(|&i| t.nodes[i].idname == edit::FRAME && hit(i)))
}

/// The socket under a point: (node, socket, is_input).
pub fn socket_at(lay: &[NodeLayout], x: i32, y: i32, zoom: f32) -> Option<(usize, usize, bool)> {
    let r2 = (9.0 * zoom.max(0.6)).powi(2);
    let mut best: Option<(f32, (usize, usize, bool))> = None;
    for (n, l) in lay.iter().enumerate().rev() {
        for (list, input) in [(&l.inputs, true), (&l.outputs, false)] {
            for (i, p) in list.iter().enumerate() {
                if let Some(p) = p {
                    let d = (p.0 - x as f32).powi(2) + (p.1 - y as f32).powi(2);
                    if d <= r2 && best.map_or(true, |b| d < b.0) {
                        best = Some((d, (n, i, input)));
                    }
                }
            }
        }
    }
    best.map(|b| b.1)
}

/// Header colour of a node (sRGB).
pub fn header_color(node: &Node) -> u32 {
    if edit::is_group_node(node) || matches!(node.idname.as_str(), edit::GROUP_INPUT | edit::GROUP_OUTPUT) {
        return sex_3d::nodes::Category::Group.header_color();
    }
    sex_3d::nodes::registry::node_type(&node.idname).map(|t| t.category.header_color()).unwrap_or(0x3b3b3b)
}
