// SPDX-License-Identifier: GPL-3.0-only
//! Node trees, shared by the shader, compositor and geometry node editors,
//! modelled on Blender's `bNodeTree`: nodes keyed by Blender's `bl_idname`
//! (`ShaderNodeBsdfPrincipled`, `ShaderNodeTexNoise`, ...), sockets by their
//! UI name and identifier, and links from an output socket to an input socket.
//!
//! The data model lives here; [`registry`] builds nodes with Blender's sockets
//! and defaults. Evaluation lives with each consumer (the path tracer's shader
//! VM, the compositor, geometry nodes).
use std::fmt;

pub mod registry;

pub use registry::{categories, legacy_node_types, new_node, node_types, sync_group_sockets, update_sockets, Category, NodeType};

/// Which editor a tree belongs to (Blender's `bl_idname` of the tree).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TreeKind {
    /// `ShaderNodeTree`: materials, worlds and lights.
    #[default]
    Shader,
    /// `CompositorNodeTree`.
    Compositor,
    /// `GeometryNodeTree`.
    Geometry,
}

impl TreeKind {
    pub fn idname(self) -> &'static str {
        match self {
            TreeKind::Shader => "ShaderNodeTree",
            TreeKind::Compositor => "CompositorNodeTree",
            TreeKind::Geometry => "GeometryNodeTree",
        }
    }
}

/// Socket data types (Blender's `NodeSocket*` families).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocketType {
    Float,
    Int,
    Bool,
    Vector,
    /// RGBA.
    Color,
    String,
    /// A closure (BSDF, emission, volume, ...).
    Shader,
    Geometry,
    Object,
    Collection,
    Image,
    Material,
    Texture,
    Rotation,
    Matrix,
    Menu,
    /// The empty "extend" socket of group input/output nodes.
    Virtual,
}

impl SocketType {
    /// Blender's socket colour (sRGB 0xRRGGBB) for drawing.
    pub fn color(self) -> u32 {
        match self {
            SocketType::Float => 0xa1a1a1,
            SocketType::Int => 0x598c5c,
            SocketType::Bool => 0xcca6d6,
            SocketType::Vector => 0x6363c7,
            SocketType::Color => 0xc7c729,
            SocketType::String => 0x70b2ff,
            SocketType::Shader => 0x63c763,
            SocketType::Geometry => 0x00d6a3,
            SocketType::Object => 0xed9e5c,
            SocketType::Collection => 0xf5f5f5,
            SocketType::Image => 0x633863,
            SocketType::Material => 0xeb7582,
            SocketType::Texture => 0x9e4fa3,
            SocketType::Rotation => 0xa663c7,
            SocketType::Matrix => 0x48a6bc,
            SocketType::Menu => 0x454545,
            SocketType::Virtual => 0x3b3b3b,
        }
    }
}

/// UI hint for a socket's value field (Blender's `PROP_FACTOR`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Subtype {
    #[default]
    None,
    /// 0..1 slider.
    Factor,
    Percentage,
    Angle,
    Distance,
    /// A direction shown as a unit vector.
    Direction,
    Euler,
    Xyz,
    Translation,
    Wavelength,
    ColorTemperature,
    FilePath,
}

/// A socket's own (unlinked) value, or a node property's value.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    None,
    Float(f32),
    Int(i32),
    Bool(bool),
    Vector([f32; 3]),
    /// Linear RGBA.
    Color([f32; 4]),
    String(String),
    /// An index into the scene's objects/collections/images/materials,
    /// depending on the socket type.
    Id(Option<usize>),
}

impl Value {
    pub fn as_float(&self) -> f32 {
        match *self {
            Value::Float(f) => f,
            Value::Int(i) => i as f32,
            Value::Bool(b) => b as i32 as f32,
            Value::Vector(v) => (v[0] + v[1] + v[2]) / 3.0,
            Value::Color(c) => 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2],
            _ => 0.0,
        }
    }

    pub fn as_vector(&self) -> [f32; 3] {
        match *self {
            Value::Vector(v) => v,
            Value::Color(c) => [c[0], c[1], c[2]],
            Value::Float(f) => [f; 3],
            Value::Int(i) => [i as f32; 3],
            Value::Bool(b) => [b as i32 as f32; 3],
            _ => [0.0; 3],
        }
    }

    pub fn as_color(&self) -> [f32; 4] {
        match *self {
            Value::Color(c) => c,
            Value::Vector(v) => [v[0], v[1], v[2], 1.0],
            Value::Float(f) => [f, f, f, 1.0],
            Value::Int(i) => [i as f32, i as f32, i as f32, 1.0],
            Value::Bool(b) => {
                let f = b as i32 as f32;
                [f, f, f, 1.0]
            }
            _ => [0.0, 0.0, 0.0, 1.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Socket {
    /// UI name ("Base Color").
    pub name: String,
    /// Stable identifier (Blender's `identifier`, e.g. "Base Color" or
    /// "Value_001" when names repeat).
    pub identifier: String,
    pub ty: SocketType,
    pub subtype: Subtype,
    pub value: Value,
    pub min: f32,
    pub max: f32,
    /// Hidden by the user (Ctrl+H).
    pub hide: bool,
    /// False when the node's current mode does not use this socket (Blender
    /// hides it, e.g. Math's second input for Sine).
    pub enabled: bool,
    /// Draw only the label, never a value field (Normal, Vector inputs that
    /// default to the shading normal or coordinates).
    pub hide_value: bool,
    /// Collapsible panel this socket belongs to (Principled's "Subsurface").
    pub panel: Option<String>,
}

impl Socket {
    pub fn new(name: &str, ty: SocketType, value: Value) -> Socket {
        Socket { name: name.into(), identifier: name.into(), ty, subtype: Subtype::None, value, min: f32::MIN, max: f32::MAX, hide: false, enabled: true, hide_value: false, panel: None }
    }

    pub fn range(mut self, min: f32, max: f32) -> Socket {
        self.min = min;
        self.max = max;
        self
    }

    pub fn subtype(mut self, s: Subtype) -> Socket {
        self.subtype = s;
        if s == Subtype::Factor && self.min == f32::MIN {
            self.min = 0.0;
            self.max = 1.0;
        }
        self
    }

    pub fn hide_value(mut self) -> Socket {
        self.hide_value = true;
        self
    }

    pub fn ident(mut self, id: &str) -> Socket {
        self.identifier = id.into();
        self
    }

    pub fn panel(mut self, p: &str) -> Socket {
        self.panel = Some(p.into());
        self
    }
}

/// A colour ramp (`ShaderNodeValToRGB`, compositor ColorRamp).
#[derive(Clone, Debug, PartialEq)]
pub struct ColorRamp {
    /// "LINEAR", "EASE", "B_SPLINE", "CARDINAL" or "CONSTANT".
    pub interpolation: String,
    /// "RGB", "HSV" or "HSL".
    pub color_mode: String,
    /// (position 0..1, linear RGBA), sorted by position.
    pub elements: Vec<(f32, [f32; 4])>,
}

impl Default for ColorRamp {
    fn default() -> Self {
        ColorRamp { interpolation: "LINEAR".into(), color_mode: "RGB".into(), elements: vec![(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])] }
    }
}

impl ColorRamp {
    /// The colour at `t`, as Blender's `BKE_colorband_evaluate`: linear,
    /// ease (smoothstep), constant (the element at or below `t`), and the
    /// B-spline and cardinal splines through the neighbouring elements.
    /// Colours are interpolated in RGB whatever `color_mode` says.
    pub fn eval(&self, t: f32) -> [f32; 4] {
        let e = &self.elements;
        if e.is_empty() {
            return [0.0, 0.0, 0.0, 1.0];
        }
        if e.len() == 1 {
            return e[0].1;
        }
        let spline = matches!(self.interpolation.as_str(), "B_SPLINE" | "CARDINAL");
        if t <= e[0].0 && !spline {
            return e[0].1;
        }
        // `a`: the first element strictly after t; `c1` = e[a], `c2` = e[a - 1].
        let a = e.iter().position(|x| x.0 > t).unwrap_or(e.len());
        let (c1, c2) = if a == e.len() {
            let last = e[e.len() - 1];
            ((1.0, last.1), last)
        } else if a == 0 {
            (e[0], (0.0, e[0].1))
        } else {
            (e[a], e[a - 1])
        };
        if self.interpolation == "CONSTANT" {
            return c2.1;
        }
        let mut f = if c2.0 != c1.0 {
            (t - c1.0) / (c2.0 - c1.0)
        } else if a != e.len() {
            0.0
        } else {
            1.0
        };
        let mut out = [0.0; 4];
        if spline {
            // Indices "from right to left: 3 2 1 0".
            let c0 = if a + 1 >= e.len() { c1 } else { e[a + 1] };
            let c3 = if a < 2 { c2 } else { e[a - 2] };
            let w = key_curve_weights(f.clamp(0.0, 1.0), self.interpolation == "CARDINAL");
            for (k, o) in out.iter_mut().enumerate() {
                *o = (w[3] * c3.1[k] + w[2] * c2.1[k] + w[1] * c1.1[k] + w[0] * c0.1[k]).clamp(0.0, 1.0);
            }
            return out;
        }
        if self.interpolation == "EASE" {
            let m = f * f;
            f = 3.0 * m - 2.0 * m * f;
        }
        for (k, o) in out.iter_mut().enumerate() {
            *o = (1.0 - f) * c1.1[k] + f * c2.1[k];
        }
        out
    }
}

/// Blender's `key_curve_position_weights` for cardinal and B-spline keys.
fn key_curve_weights(t: f32, cardinal: bool) -> [f32; 4] {
    let (t2, t3) = (t * t, t * t * t);
    if cardinal {
        let fc = 0.71;
        [-fc * t3 + 2.0 * fc * t2 - fc * t, (2.0 - fc) * t3 + (fc - 3.0) * t2 + 1.0, (fc - 2.0) * t3 + (3.0 - 2.0 * fc) * t2 + fc * t, fc * t3 - fc * t2]
    } else {
        [-t3 / 6.0 + 0.5 * t2 - 0.5 * t + 1.0 / 6.0, 0.5 * t3 - t2 + 2.0 / 3.0, -0.5 * t3 + 0.5 * t2 + 0.5 * t + 1.0 / 6.0, t3 / 6.0]
    }
}

/// RGB/vector/float curves (`ShaderNodeRGBCurve`, `ShaderNodeVectorCurve`,
/// `ShaderNodeFloatCurve`): up to four curves of (x, y) control points with
/// Blender's automatic Bezier handles. RGB curves store (C, R, G, B), vector
/// curves (X, Y, Z), float curves one curve.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveMapping {
    pub curves: Vec<Vec<[f32; 2]>>,
}

impl Default for CurveMapping {
    fn default() -> Self {
        CurveMapping { curves: vec![vec![[0.0, 0.0], [1.0, 1.0]]; 4] }
    }
}

impl CurveMapping {
    /// Evaluate curve `i` at `x` (identity when the curve is missing; flat
    /// beyond the end points, like Blender without extrapolation).
    pub fn eval(&self, i: usize, x: f32) -> f32 {
        match self.curves.get(i) {
            Some(c) => polyline_eval(&curve_polyline(c), x),
            None => x,
        }
    }

    /// Curve `i` flattened to a polyline (Blender bakes its `table` alike).
    pub fn polyline(&self, i: usize) -> Option<Vec<[f32; 2]>> {
        self.curves.get(i).map(|c| curve_polyline(c))
    }
}

/// Evaluate a polyline sorted by x; flat outside its ends, identity when empty.
pub fn polyline_eval(poly: &[[f32; 2]], x: f32) -> f32 {
    match poly {
        [] => x,
        [p] => p[1],
        _ => {
            if x <= poly[0][0] {
                return poly[0][1];
            }
            let last = poly[poly.len() - 1];
            if x >= last[0] {
                return last[1];
            }
            let i = poly.partition_point(|p| p[0] <= x).clamp(1, poly.len() - 1);
            let (a, b) = (poly[i - 1], poly[i]);
            let f = if b[0] > a[0] { (x - a[0]) / (b[0] - a[0]) } else { 0.0 };
            a[1] + (b[1] - a[1]) * f
        }
    }
}

type V2 = [f32; 2];

fn sub2(a: V2, b: V2) -> V2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn len2(a: V2) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}

/// The handle pair of an end point re-aimed at its neighbour's handle
/// (`calchandles_curvemap`), keeping the handle length.
fn aim_end(p: V2, target: V2, handle_len: f32, rightward: bool) -> Option<(V2, V2)> {
    let mut v = target;
    if (rightward && v[0] < p[0]) || (!rightward && v[0] > p[0]) {
        v[0] = p[0];
    }
    let d = sub2(v, p);
    let nl = len2(d);
    (nl > f32::EPSILON).then(|| {
        let d = [d[0] * handle_len / nl, d[1] * handle_len / nl];
        let (back, fwd) = ([p[0] - d[0], p[1] - d[1]], [p[0] + d[0], p[1] + d[1]]);
        if rightward {
            (back, fwd)
        } else {
            (fwd, back)
        }
    })
}

/// Points of a curve with Blender's automatic handles (`calchandle_curvemap`),
/// flattened into a polyline of 16 steps per Bezier segment.
fn curve_polyline(c: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut pts: Vec<V2> = c.to_vec();
    pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let n = pts.len();
    if n < 2 {
        return pts;
    }
    // (left handle, right handle) per point.
    let mut h: Vec<(V2, V2)> = Vec::with_capacity(n);
    for i in 0..n {
        let p2 = pts[i];
        let p1 = if i > 0 { pts[i - 1] } else { [2.0 * p2[0] - pts[1][0], 2.0 * p2[1] - pts[1][1]] };
        let p3 = if i + 1 < n { pts[i + 1] } else { [2.0 * p2[0] - pts[n - 2][0], 2.0 * p2[1] - pts[n - 2][1]] };
        let (da, db) = (sub2(p2, p1), sub2(p3, p2));
        let (la, lb) = (len2(da).max(1e-12), len2(db).max(1e-12));
        let t = [db[0] / lb + da[0] / la, db[1] / lb + da[1] / la];
        let l = len2(t) * 2.5614;
        if l > 0.0 {
            let (sa, sb) = (la / l, lb / l);
            h.push(([p2[0] - t[0] * sa, p2[1] - t[1] * sa], [p2[0] + t[0] * sb, p2[1] + t[1] * sb]));
        } else {
            h.push((p2, p2));
        }
    }
    if let Some(x) = aim_end(pts[0], h[1].0, len2(sub2(h[0].1, pts[0])), true) {
        h[0] = x;
    }
    if let Some(x) = aim_end(pts[n - 1], h[n - 2].1, len2(sub2(h[n - 1].0, pts[n - 1])), false) {
        h[n - 1] = x;
    }
    let mut out = Vec::with_capacity((n - 1) * 16 + 1);
    out.push(pts[0]);
    for i in 0..n - 1 {
        let (a, d) = (pts[i], pts[i + 1]);
        // Keep x monotonic inside the segment (Blender's `correct_bezpart`).
        let clampx = |v: V2| [v[0].clamp(a[0], d[0]), v[1]];
        let (b, c) = (clampx(h[i].1), clampx(h[i + 1].0));
        for s in 1..=16 {
            let t = s as f32 / 16.0;
            let u = 1.0 - t;
            let w = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            out.push([w[0] * a[0] + w[1] * b[0] + w[2] * c[0] + w[3] * d[0], w[0] * a[1] + w[1] * b[1] + w[2] * c[1] + w[3] * d[1]]);
        }
    }
    out
}

/// A node property that is not a socket (dropdowns, checkboxes, ramps).
#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    /// An enum, stored as Blender's RNA identifier ("MULTIPLY", "GGX").
    Enum(String),
    Bool(bool),
    Int(i32),
    Float(f32),
    Vector([f32; 3]),
    Color([f32; 4]),
    String(String),
    Ramp(ColorRamp),
    Curve(CurveMapping),
    /// Image, object, node group ... index into the scene.
    Id(Option<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Unique within the tree ("Principled BSDF", "Math.001").
    pub name: String,
    /// Blender's `bl_idname`.
    pub idname: String,
    /// User label; empty shows the type's label.
    pub label: String,
    /// Canvas position of the top-left corner (Blender's node space, y up).
    pub location: [f32; 2],
    pub width: f32,
    pub inputs: Vec<Socket>,
    pub outputs: Vec<Socket>,
    /// Properties by RNA name ("operation", "blend_type", "color_ramp").
    pub props: Vec<(String, Prop)>,
    pub mute: bool,
    /// Collapsed (H).
    pub hide: bool,
    /// The frame node this node sits in.
    pub parent: Option<usize>,
    /// Custom header colour (linear RGB).
    pub color: Option<[f32; 3]>,
    pub select: bool,
}

impl Node {
    pub fn new(idname: &str, name: &str) -> Node {
        Node { name: name.into(), idname: idname.into(), label: String::new(), location: [0.0, 0.0], width: 140.0, inputs: Vec::new(), outputs: Vec::new(), props: Vec::new(), mute: false, hide: false, parent: None, color: None, select: false }
    }

    pub fn input(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|s| s.identifier == name).or_else(|| self.inputs.iter().position(|s| s.name == name && s.enabled))
    }

    pub fn output(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|s| s.identifier == name).or_else(|| self.outputs.iter().position(|s| s.name == name && s.enabled))
    }

    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.0 == name).map(|p| &p.1)
    }

    pub fn prop_mut(&mut self, name: &str) -> Option<&mut Prop> {
        self.props.iter_mut().find(|p| p.0 == name).map(|p| &mut p.1)
    }

    pub fn set_prop(&mut self, name: &str, v: Prop) {
        match self.prop_mut(name) {
            Some(p) => *p = v,
            None => self.props.push((name.into(), v)),
        }
    }

    /// An enum property's identifier, or "".
    pub fn enum_prop(&self, name: &str) -> &str {
        match self.prop(name) {
            Some(Prop::Enum(s)) => s,
            _ => "",
        }
    }

    pub fn bool_prop(&self, name: &str) -> bool {
        matches!(self.prop(name), Some(Prop::Bool(true)))
    }

    /// Set an input's own value by name.
    pub fn set_input(&mut self, name: &str, v: Value) -> bool {
        match self.input(name) {
            Some(i) => {
                self.inputs[i].value = v;
                true
            }
            None => false,
        }
    }

    /// The label to draw in the header.
    pub fn display_label(&self) -> String {
        if !self.label.is_empty() {
            return self.label.clone();
        }
        registry::node_type(&self.idname).map(|t| t.label.to_string()).unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Link {
    pub from_node: usize,
    pub from_socket: usize,
    pub to_node: usize,
    pub to_socket: usize,
    pub muted: bool,
}

/// A group's interface socket (Blender's `NodeTreeInterfaceSocket`).
#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceSocket {
    pub name: String,
    pub ty: SocketType,
    pub default: Value,
    pub min: f32,
    pub max: f32,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct NodeTree {
    pub name: String,
    pub kind: TreeKind,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    /// Group inputs/outputs (for node groups; empty for material trees).
    pub inputs: Vec<InterfaceSocket>,
    pub outputs: Vec<InterfaceSocket>,
    /// The node shown in the editor's sidebar / last clicked.
    pub active: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    NoSuchNode,
    NoSuchSocket,
    /// Would create a cycle.
    Cycle,
    SameNode,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LinkError::NoSuchNode => "no such node",
            LinkError::NoSuchSocket => "no such socket",
            LinkError::Cycle => "link would create a cycle",
            LinkError::SameNode => "cannot link a node to itself",
        })
    }
}

impl NodeTree {
    pub fn new(name: &str, kind: TreeKind) -> NodeTree {
        NodeTree { name: name.into(), kind, ..Default::default() }
    }

    /// A unique node name from `base` ("Math", "Math.001", ...).
    pub fn unique_name(&self, base: &str) -> String {
        if !self.nodes.iter().any(|n| n.name == base) {
            return base.into();
        }
        (1..).map(|i| format!("{base}.{i:03}")).find(|n| !self.nodes.iter().any(|x| &x.name == n)).unwrap()
    }

    /// Add a registered node type at `location`; returns its index.
    pub fn add(&mut self, idname: &str, location: [f32; 2]) -> Option<usize> {
        let mut n = new_node(idname)?;
        n.name = self.unique_name(&n.name);
        n.location = location;
        self.nodes.push(n);
        Some(self.nodes.len() - 1)
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// The first node of type `idname`.
    pub fn find_type(&self, idname: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.idname == idname)
    }

    /// Link `from_node`'s output `from` to `to_node`'s input `to` (socket
    /// identifiers or names). An input takes one link, so an existing link
    /// into it is replaced. Refuses links that would make a cycle.
    pub fn link(&mut self, from_node: usize, from: &str, to_node: usize, to: &str) -> Result<usize, LinkError> {
        let fs = self.nodes.get(from_node).ok_or(LinkError::NoSuchNode)?.output(from).ok_or(LinkError::NoSuchSocket)?;
        let ts = self.nodes.get(to_node).ok_or(LinkError::NoSuchNode)?.input(to).ok_or(LinkError::NoSuchSocket)?;
        self.link_sockets(from_node, fs, to_node, ts)
    }

    pub fn link_sockets(&mut self, from_node: usize, from_socket: usize, to_node: usize, to_socket: usize) -> Result<usize, LinkError> {
        if from_node == to_node {
            return Err(LinkError::SameNode);
        }
        if from_node >= self.nodes.len() || to_node >= self.nodes.len() {
            return Err(LinkError::NoSuchNode);
        }
        if from_socket >= self.nodes[from_node].outputs.len() || to_socket >= self.nodes[to_node].inputs.len() {
            return Err(LinkError::NoSuchSocket);
        }
        if self.depends_on(from_node, to_node) {
            return Err(LinkError::Cycle);
        }
        self.links.retain(|l| !(l.to_node == to_node && l.to_socket == to_socket));
        self.links.push(Link { from_node, from_socket, to_node, to_socket, muted: false });
        Ok(self.links.len() - 1)
    }

    /// Remove the link into `to_node`'s input `to_socket`, if any.
    pub fn unlink_input(&mut self, to_node: usize, to_socket: usize) -> bool {
        let n = self.links.len();
        self.links.retain(|l| !(l.to_node == to_node && l.to_socket == to_socket));
        n != self.links.len()
    }

    /// Does `node` (transitively) read from `other`?
    pub fn depends_on(&self, node: usize, other: usize) -> bool {
        let mut stack = vec![node];
        let mut seen = vec![false; self.nodes.len()];
        while let Some(n) = stack.pop() {
            if n == other {
                return true;
            }
            if std::mem::replace(&mut seen[n], true) {
                continue;
            }
            stack.extend(self.links.iter().filter(|l| l.to_node == n).map(|l| l.from_node));
        }
        false
    }

    /// The link feeding `node`'s input `socket`.
    pub fn input_link(&self, node: usize, socket: usize) -> Option<&Link> {
        self.links.iter().find(|l| l.to_node == node && l.to_socket == socket && !l.muted)
    }

    /// Remove a node and its links, keeping the other indices valid.
    pub fn remove(&mut self, node: usize) {
        if node >= self.nodes.len() {
            return;
        }
        self.nodes.remove(node);
        self.links.retain(|l| l.from_node != node && l.to_node != node);
        for l in &mut self.links {
            if l.from_node > node {
                l.from_node -= 1;
            }
            if l.to_node > node {
                l.to_node -= 1;
            }
        }
        for n in &mut self.nodes {
            n.parent = match n.parent {
                Some(p) if p == node => None,
                Some(p) if p > node => Some(p - 1),
                p => p,
            };
        }
        self.active = match self.active {
            Some(a) if a == node => None,
            Some(a) if a > node => Some(a - 1),
            a => a,
        };
    }

    /// The active output node of type `idname` (Blender picks the one with
    /// `is_active_output`; we take the last selected-or-first).
    pub fn output_node(&self, idname: &str) -> Option<usize> {
        let mut found = None;
        for (i, n) in self.nodes.iter().enumerate() {
            if n.idname == idname {
                if found.is_none() || n.bool_prop("is_active_output") {
                    found = Some(i);
                }
            }
        }
        found
    }

    /// Nodes in an order where every node comes after the nodes it reads.
    pub fn topo_order(&self) -> Vec<usize> {
        let n = self.nodes.len();
        let mut state = vec![0u8; n];
        let mut out = Vec::with_capacity(n);
        fn visit(t: &NodeTree, i: usize, state: &mut [u8], out: &mut Vec<usize>) {
            if state[i] != 0 {
                return;
            }
            state[i] = 1;
            for l in t.links.iter().filter(|l| l.to_node == i) {
                visit(t, l.from_node, state, out);
            }
            state[i] = 2;
            out.push(i);
        }
        for i in 0..n {
            visit(self, i, &mut state, &mut out);
        }
        out
    }

    /// Blender's default material tree: Principled BSDF into Material Output.
    pub fn default_material(name: &str) -> NodeTree {
        let mut t = NodeTree::new(name, TreeKind::Shader);
        let p = t.add("ShaderNodeBsdfPrincipled", [-200.0, 300.0]).unwrap();
        let o = t.add("ShaderNodeOutputMaterial", [300.0, 300.0]).unwrap();
        t.link(p, "BSDF", o, "Surface").unwrap();
        t
    }

    /// Blender's default world tree: Background into World Output.
    pub fn default_world(name: &str) -> NodeTree {
        let mut t = NodeTree::new(name, TreeKind::Shader);
        let b = t.add("ShaderNodeBackground", [-200.0, 300.0]).unwrap();
        let o = t.add("ShaderNodeOutputWorld", [100.0, 300.0]).unwrap();
        t.link(b, "Background", o, "Surface").unwrap();
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_material_links_principled_to_output() {
        let t = NodeTree::default_material("Material");
        let o = t.find_type("ShaderNodeOutputMaterial").unwrap();
        let s = t.nodes[o].input("Surface").unwrap();
        let l = t.input_link(o, s).unwrap();
        assert_eq!(t.nodes[l.from_node].idname, "ShaderNodeBsdfPrincipled");
        assert_eq!(t.topo_order().last(), Some(&o));
    }

    #[test]
    fn links_replace_and_refuse_cycles() {
        let mut t = NodeTree::new("T", TreeKind::Shader);
        let a = t.add("ShaderNodeMath", [0.0, 0.0]).unwrap();
        let b = t.add("ShaderNodeMath", [0.0, 0.0]).unwrap();
        assert_eq!(t.nodes[b].name, "Math.001");
        t.link_sockets(a, 0, b, 0).unwrap();
        t.link_sockets(a, 0, b, 0).unwrap();
        assert_eq!(t.links.len(), 1);
        assert_eq!(t.link_sockets(b, 0, a, 0), Err(LinkError::Cycle));
        t.remove(a);
        assert!(t.links.is_empty());
        assert_eq!(t.nodes[0].name, "Math.001");
    }

    #[test]
    fn ramp_interpolates() {
        let r = ColorRamp::default();
        assert!((r.eval(0.25)[0] - 0.25).abs() < 1e-6);
        let mut c = r.clone();
        c.interpolation = "CONSTANT".into();
        assert_eq!(c.eval(0.9)[0], 0.0);
    }
}
