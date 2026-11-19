// SPDX-License-Identifier: GPL-3.0-only
//! The geometry node tree evaluator.
//!
//! Like Blender's lazy-function graph, values flow along links from the
//! Group Output backwards: each node is evaluated once (memoised) from the
//! values of its inputs. Data sockets carry fields ([`Field`]); a function
//! node whose inputs are all constants folds to a constant, otherwise it
//! builds a field that geometry nodes evaluate on their own geometry.
//!
//! Zones: a Repeat zone (Repeat Input / Repeat Output) re-evaluates the
//! nodes between the pair `Iterations` times, feeding each iteration's
//! outputs back as the next iteration's inputs. A Simulation zone
//! (Simulation Input / Simulation Output) carries its state from frame to
//! frame: the state of every evaluated frame is cached in memory (keyed by
//! object, modifier and zone), the start frame resets it, and evaluating a
//! later frame steps forward from the last cached one.
use super::attributes::{AttrType, Domain, Val};
use super::field::{self, eval_single, konst, map, Field, FieldNode, FnOp};
use super::func::{self, convert_to, value_to_val};
use super::{realize, Geometry, InstanceRef};
use crate::math::Vec3;
use crate::mesh::Mesh;
use crate::nodes::{edit, Node, NodeTree, Prop, SocketType, Value};
use crate::scene::{ObjectData, Scene};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

/// A socket's value during evaluation.
#[derive(Clone, Debug, Default)]
pub enum SocketValue {
    #[default]
    None,
    Geometry(Geometry),
    /// Data sockets (float, int, bool, vector, colour, rotation, matrix):
    /// a field, constant when it does not depend on geometry.
    Field(Field),
    String(String),
    /// Object, collection, material or image: an index into the scene.
    Id(Option<usize>),
    /// The links into a multi-input socket (Join Geometry), in order.
    List(Vec<SocketValue>),
}

impl SocketValue {
    pub fn single(v: Val) -> SocketValue {
        SocketValue::Field(konst(v))
    }

    pub fn field(&self) -> Field {
        match self {
            SocketValue::Field(f) => f.clone(),
            SocketValue::List(l) => l.first().map_or_else(|| konst(Val::Float(0.0)), |x| x.field()),
            _ => konst(Val::Float(0.0)),
        }
    }

    /// The value without geometry context.
    pub fn val(&self) -> Val {
        eval_single(&self.field())
    }

    pub fn geometry(&self) -> Geometry {
        match self {
            SocketValue::Geometry(g) => g.clone(),
            SocketValue::List(l) => {
                let mut g = Geometry::default();
                for x in l {
                    g.join(x.geometry());
                }
                g
            }
            _ => Geometry::default(),
        }
    }

    pub fn string(&self) -> String {
        match self {
            SocketValue::String(s) => s.clone(),
            SocketValue::List(l) => l.iter().map(|x| x.string()).collect::<Vec<_>>().join(""),
            _ => String::new(),
        }
    }

    pub fn id(&self) -> Option<usize> {
        match self {
            SocketValue::Id(i) => *i,
            _ => None,
        }
    }

    pub fn is_field(&self) -> bool {
        matches!(self, SocketValue::Field(f) if !f.is_const())
    }
}

/// What a tree is evaluated for.
#[derive(Clone, Copy, Default)]
pub struct EvalOptions<'a> {
    pub scene: Option<&'a Scene>,
    pub frame: f32,
    /// The object the modifier is on (Self Object, simulation cache key).
    pub self_object: Option<usize>,
    /// Modifier recursion depth (objects instancing objects).
    pub depth: u32,
}

impl<'a> EvalOptions<'a> {
    pub fn groups(&self) -> &'a [NodeTree] {
        self.scene.map_or(&[], |s| s.node_groups.as_slice())
    }

    /// Geometry of an instance reference (objects and collections).
    pub fn resolve(&self, r: &InstanceRef) -> Option<Geometry> {
        let s = self.scene?;
        match *r {
            InstanceRef::Geometry(ref g) => Some((**g).clone()),
            InstanceRef::Object(i) => object_geometry(s, i, self.self_object, self.depth),
            InstanceRef::Collection(c) => {
                let mut g = Geometry::default();
                for (i, o) in s.objects.iter().enumerate() {
                    if o.collection == c && o.parent.is_none_or(|p| s.objects[p].collection != c) {
                        if let Some(mut x) = object_geometry(s, i, self.self_object, self.depth) {
                            x.transform(&s.world_matrix(i));
                            g.join(x);
                        }
                    }
                }
                Some(g)
            }
        }
    }
}

/// An object's evaluated geometry in its own space.
pub fn object_geometry(s: &Scene, i: usize, self_object: Option<usize>, depth: u32) -> Option<Geometry> {
    let o = s.objects.get(i)?;
    match o.data {
        ObjectData::Mesh(mi) => {
            let m = if Some(i) == self_object || depth >= 3 { s.meshes.get(mi)?.clone() } else { s.evaluated_mesh_depth(i, depth + 1)? };
            Some(Geometry::from_mesh(m))
        }
        _ => Some(Geometry::default()),
    }
}

/// The node inputs as evaluated values, looked up by identifier or name.
pub(crate) struct Ins<'n> {
    pub node: &'n Node,
    pub vals: Vec<SocketValue>,
}

impl<'n> Ins<'n> {
    fn idx(&self, id: &str) -> Option<usize> {
        self.node.input(id).or_else(|| self.node.inputs.iter().position(|s| s.name == id))
    }

    pub fn get(&self, id: &str) -> &SocketValue {
        static NONE: SocketValue = SocketValue::None;
        self.idx(id).and_then(|i| self.vals.get(i)).unwrap_or(&NONE)
    }

    pub fn geo(&self, id: &str) -> Geometry {
        self.get(id).geometry()
    }

    pub fn field(&self, id: &str) -> Field {
        match self.get(id) {
            SocketValue::None => {
                let s = self.idx(id).map(|i| &self.node.inputs[i]);
                konst(s.map_or(Val::Float(0.0), |s| value_to_val(&s.value, s.ty)))
            }
            v => v.field(),
        }
    }

    pub fn val(&self, id: &str) -> Val {
        eval_single(&self.field(id))
    }

    pub fn f(&self, id: &str) -> f32 {
        self.val(id).f()
    }

    pub fn i(&self, id: &str) -> i32 {
        self.val(id).i()
    }

    pub fn b(&self, id: &str) -> bool {
        self.val(id).b()
    }

    pub fn v(&self, id: &str) -> Vec3 {
        self.val(id).v()
    }

    pub fn s(&self, id: &str) -> String {
        self.get(id).string()
    }

    pub fn id(&self, id: &str) -> Option<usize> {
        self.get(id).id()
    }

    pub fn list(&self, id: &str) -> Vec<SocketValue> {
        match self.get(id) {
            SocketValue::List(l) => l.clone(),
            SocketValue::None => Vec::new(),
            v => vec![v.clone()],
        }
    }
}

/// Outputs in socket order, set by identifier or name.
pub(crate) struct Outs<'n> {
    node: &'n Node,
    pub vals: Vec<SocketValue>,
}

impl<'n> Outs<'n> {
    pub fn new(node: &'n Node) -> Outs<'n> {
        Outs { node, vals: vec![SocketValue::None; node.outputs.len()] }
    }

    pub fn set(&mut self, id: &str, v: SocketValue) -> &mut Self {
        if let Some(i) = self.node.output(id).or_else(|| self.node.outputs.iter().position(|s| s.name == id)) {
            self.vals[i] = v;
        }
        self
    }

    pub fn geo(&mut self, id: &str, g: Geometry) -> &mut Self {
        self.set(id, SocketValue::Geometry(g))
    }

    pub fn field(&mut self, id: &str, f: Field) -> &mut Self {
        self.set(id, SocketValue::Field(f))
    }

    pub fn val(&mut self, id: &str, v: Val) -> &mut Self {
        self.set(id, SocketValue::single(v))
    }

    pub fn done(self) -> Vec<SocketValue> {
        self.vals
    }
}

/// Context for node implementations.
pub(crate) struct NodeCx<'a> {
    pub opts: &'a EvalOptions<'a>,
}

/// Unlinked inputs that read the context instead of a constant (Blender's
/// implicit field inputs: Position for Set Position and textures, Index
/// for sampling, ID for Random Value, ...).
fn implicit_input(idname: &str, id: &str) -> Option<Field> {
    Some(match (idname, id) {
        ("GeometryNodeSetPosition", "Position") => field::position(),
        (t, "Vector") if t.starts_with("ShaderNodeTex") => field::position(),
        ("FunctionNodeRandomValue", "ID") => field::id(),
        ("GeometryNodeSetID", "ID") => field::index(),
        ("GeometryNodeSampleIndex", "Index") => field::index(),
        ("GeometryNodeSampleNearest" | "GeometryNodeSampleNearestSurface", "Sample Position") => field::position(),
        ("GeometryNodeRaycast", "Source Position") => field::position(),
        ("GeometryNodeProximity", "Source Position") => field::position(),
        ("GeometryNodeCornersOfFace", "Face Index") | ("GeometryNodeCornersOfVertex", "Vertex Index") | ("GeometryNodeEdgesOfVertex", "Vertex Index") => field::index(),
        ("GeometryNodeEdgesOfCorner" | "GeometryNodeFaceOfCorner" | "GeometryNodeOffsetCornerInFace" | "GeometryNodeVertexOfCorner", "Corner Index") => field::index(),
        ("GeometryNodeCurveOfPoint" | "GeometryNodeOffsetPointInCurve", "Point Index") | ("GeometryNodePointsOfCurve", "Curve Index") => field::index(),
        ("GeometryNodeSampleCurve", "Curve Index") => konst(Val::Int(0)),
        ("GeometryNodeExtrudeMesh", "Offset") => field::normal(),
        ("GeometryNodeScaleElements", "Center") | ("GeometryNodeMeshToPoints" | "GeometryNodeInstancesToPoints", "Position") => field::position(),
        // Set Handle Positions reads the handle being set (see nodes_curve).
        ("GeometryNodeSetCurveHandlePositions", "Position") => field::input("Handle Position Implicit", Val::Vector(Vec3::ZERO), |_, _| None),
        _ => return None,
    })
}

/// Convert a value to what socket type `ty` holds.
pub(crate) fn convert_value(v: SocketValue, ty: SocketType) -> SocketValue {
    let data = matches!(ty, SocketType::Float | SocketType::Int | SocketType::Bool | SocketType::Vector | SocketType::Color | SocketType::Rotation | SocketType::Matrix);
    match v {
        SocketValue::Field(f) if data => {
            let want = convert_to(f.default_val(), ty);
            if AttrType::of_val(&want) == AttrType::of_val(&f.default_val()) {
                return SocketValue::Field(f);
            }
            if let FieldNode::Const(c) = &*f {
                return SocketValue::single(convert_to(*c, ty));
            }
            SocketValue::Field(map(vec![f], want, move |x| convert_to(x[0], ty)))
        }
        SocketValue::List(l) => SocketValue::List(l.into_iter().map(|x| convert_value(x, ty)).collect()),
        other => other,
    }
}

fn own_value(s: &crate::nodes::Socket) -> SocketValue {
    match s.ty {
        SocketType::Geometry => SocketValue::Geometry(Geometry::default()),
        SocketType::String | SocketType::Menu => SocketValue::String(match &s.value {
            Value::String(x) => x.clone(),
            _ => String::new(),
        }),
        SocketType::Object | SocketType::Collection | SocketType::Material | SocketType::Image | SocketType::Texture => SocketValue::Id(match s.value {
            Value::Id(i) => i,
            _ => None,
        }),
        SocketType::Virtual | SocketType::Shader => SocketValue::None,
        ty => SocketValue::single(value_to_val(&s.value, ty)),
    }
}

/// One evaluation of a tree (a group call).
struct Run<'t> {
    tree: &'t NodeTree,
    inputs: Vec<SocketValue>,
    memo: HashMap<usize, Vec<SocketValue>>,
    overrides: HashMap<usize, Vec<SocketValue>>,
    /// For simulation cache keys.
    path: String,
    depth: u32,
}

pub(crate) fn is_multi_input(idname: &str, id: &str) -> bool {
    crate::nodes::registry::is_multi_input(idname, id)
}

struct Evaluator<'a> {
    opts: &'a EvalOptions<'a>,
    groups: &'a [NodeTree],
}

type SimStore = HashMap<String, BTreeMap<i32, Vec<SocketValue>>>;

fn sim_store() -> &'static Mutex<SimStore> {
    static S: OnceLock<Mutex<SimStore>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Forget every cached simulation frame.
pub fn clear_simulation_cache() {
    if let Ok(mut s) = sim_store().lock() {
        s.clear();
    }
}

/// The zone partner of a zone input node: `paired_output` names it, else
/// the zone outputs of that kind pair with the zone inputs in node order.
fn paired_output(t: &NodeTree, input: usize, out_idname: &str) -> Option<usize> {
    if let Some(Prop::String(name)) = t.nodes[input].prop("paired_output") {
        if let Some(i) = t.find(name).filter(|&i| t.nodes[i].idname == out_idname) {
            return Some(i);
        }
    }
    let in_idname = t.nodes[input].idname.clone();
    let k = t.nodes[..input].iter().filter(|n| n.idname == in_idname).count();
    t.nodes.iter().enumerate().filter(|(_, n)| n.idname == out_idname).nth(k).map(|(i, _)| i)
}

fn paired_input(t: &NodeTree, output: usize, in_idname: &str) -> Option<usize> {
    (0..t.nodes.len()).find(|&i| t.nodes[i].idname == in_idname && paired_output(t, i, &t.nodes[output].idname) == Some(output))
}

impl<'a> Evaluator<'a> {
    fn eval_tree(&self, tree: &NodeTree, inputs: Vec<SocketValue>, path: &str, depth: u32) -> Vec<SocketValue> {
        let mut run = Run { tree, inputs, memo: HashMap::new(), overrides: HashMap::new(), path: path.to_string(), depth };
        let out = tree.output_node(edit::GROUP_OUTPUT);
        let mut res: Vec<SocketValue> = tree.outputs.iter().map(|s| own_value(&crate::nodes::Socket::new(&s.name, s.ty, s.default.clone()))).collect();
        if let Some(o) = out {
            for (k, iface) in tree.outputs.iter().enumerate() {
                if k < tree.nodes[o].inputs.len() {
                    let v = self.input(&mut run, o, k);
                    res[k] = convert_value(v, iface.ty);
                }
            }
        }
        res
    }

    /// The value flowing into input `si` of node `ni`.
    fn input(&self, run: &mut Run, ni: usize, si: usize) -> SocketValue {
        let tree = run.tree;
        let node = &tree.nodes[ni];
        let sock = &node.inputs[si];
        let links: Vec<(usize, usize)> = tree.links.iter().filter(|l| l.to_node == ni && l.to_socket == si && !l.muted).map(|l| (l.from_node, l.from_socket)).collect();
        if links.is_empty() {
            if let Some(f) = implicit_input(&node.idname, &sock.identifier) {
                return convert_value(SocketValue::Field(f), sock.ty);
            }
            if is_multi_input(&node.idname, &sock.identifier) {
                return SocketValue::List(Vec::new());
            }
            return own_value(sock);
        }
        let mut vals: Vec<SocketValue> = links.iter().map(|&(fnode, fs)| self.outputs(run, fnode).get(fs).cloned().unwrap_or_default()).map(|v| convert_value(v, sock.ty)).collect();
        if is_multi_input(&node.idname, &sock.identifier) {
            return SocketValue::List(vals);
        }
        vals.pop().unwrap_or_default()
    }

    fn outputs(&self, run: &mut Run, ni: usize) -> Vec<SocketValue> {
        if let Some(o) = run.overrides.get(&ni) {
            return o.clone();
        }
        if let Some(o) = run.memo.get(&ni) {
            return o.clone();
        }
        let out = self.exec(run, ni);
        run.memo.insert(ni, out.clone());
        out
    }

    fn exec(&self, run: &mut Run, ni: usize) -> Vec<SocketValue> {
        let tree = run.tree;
        let node = &tree.nodes[ni];
        let nout = node.outputs.len();
        let pad = |mut v: Vec<SocketValue>| {
            v.resize(nout, SocketValue::None);
            v
        };
        match node.idname.as_str() {
            edit::GROUP_INPUT => {
                return pad(node.outputs.iter().enumerate().map(|(k, s)| run.inputs.get(k).cloned().map(|v| convert_value(v, s.ty)).unwrap_or_default()).collect());
            }
            edit::FRAME => return pad(Vec::new()),
            "GeometryNodeRepeatOutput" => return pad(self.repeat(run, ni)),
            "GeometryNodeSimulationOutput" => return pad(self.simulation(run, ni)),
            "GeometryNodeRepeatInput" | "GeometryNodeSimulationInput" => {
                // Outside an iteration: the initial values pass through.
                let items: Vec<SocketValue> = self.zone_initial(run, ni);
                let first = if node.idname == "GeometryNodeRepeatInput" { SocketValue::single(Val::Int(0)) } else { SocketValue::single(Val::Float(0.0)) };
                return pad(std::iter::once(first).chain(items).collect());
            }
            _ => {}
        }
        let vals: Vec<SocketValue> = (0..node.inputs.len()).map(|si| self.input(run, ni, si)).collect();
        if node.mute || node.idname == edit::REROUTE {
            let mut out = vec![SocketValue::None; nout];
            if node.idname == edit::REROUTE {
                if let Some(v) = vals.into_iter().next() {
                    out[0] = v;
                }
                return out;
            }
            for (i, o) in edit::internal_links(tree, ni) {
                out[o] = convert_value(vals[i].clone(), node.outputs[o].ty);
            }
            return out;
        }
        if let Some(gi) = edit::group_of(node) {
            let Some(g) = self.groups.get(gi) else { return pad(Vec::new()) };
            if run.depth > 32 {
                return pad(Vec::new());
            }
            let path = format!("{}/{}", run.path, node.name);
            return pad(self.eval_tree(g, vals, &path, run.depth + 1));
        }
        let ins = Ins { node, vals };
        pad(self.exec_node(ins))
    }

    fn exec_node(&self, ins: Ins) -> Vec<SocketValue> {
        let node = ins.node;
        let cx = NodeCx { opts: self.opts };
        // Constants.
        match node.idname.as_str() {
            "ShaderNodeValue" | "FunctionNodeInputVector" | "FunctionNodeInputInt" | "FunctionNodeInputBool" | "FunctionNodeInputColor" | "FunctionNodeInputRotation" | "GeometryNodeInputVector" => {
                return constant(node).into_iter().collect();
            }
            "FunctionNodeInputString" => {
                let s = match node.prop("string") {
                    Some(Prop::String(s)) => s.clone(),
                    _ => String::new(),
                };
                return vec![SocketValue::String(s)];
            }
            "GeometryNodeSwitch" | "GeometryNodeIndexSwitch" | "GeometryNodeMenuSwitch" => {
                let data = node.outputs.first().is_some_and(|s| matches!(s.ty, SocketType::Float | SocketType::Int | SocketType::Bool | SocketType::Vector | SocketType::Color | SocketType::Rotation | SocketType::Matrix));
                if !data || node.idname == "GeometryNodeMenuSwitch" || !ins.get(if node.idname == "GeometryNodeSwitch" { "Switch" } else { "Index" }).is_field() {
                    return switch_single(&ins);
                }
            }
            _ => {}
        }
        if let Some(f) = func::node_fn(node) {
            let fields: Vec<Field> = ins.vals.iter().zip(&node.inputs).map(|(v, s)| if matches!(v, SocketValue::Field(_)) { v.field() } else { konst(value_to_val(&s.value, s.ty)) }).collect();
            let defaults: Vec<Val> = node.outputs.iter().map(|s| value_to_val(&s.value, s.ty)).collect();
            if fields.iter().all(|x| x.is_const()) {
                let args: Vec<Val> = fields.iter().map(|x| x.default_val()).collect();
                let res = f(&args);
                return node.outputs.iter().enumerate().map(|(k, s)| SocketValue::single(convert_to(res.get(k).copied().unwrap_or(defaults[k]), s.ty))).collect();
            }
            let op = Arc::new(FnOp { f, inputs: fields });
            return node.outputs.iter().enumerate().map(|(k, s)| SocketValue::Field(Arc::new(FieldNode::Fn { op: op.clone(), output: k, default: convert_to(defaults[k], s.ty) }))).collect();
        }
        if let Some(v) = super::nodes_geo::exec(&cx, &ins) {
            return v;
        }
        if let Some(v) = super::nodes_mesh::exec(&cx, &ins) {
            return v;
        }
        if let Some(v) = super::nodes_curve::exec(&cx, &ins) {
            return v;
        }
        // Unknown nodes pass their first geometry through.
        let g = ins.vals.iter().zip(&node.inputs).find(|(_, s)| s.ty == SocketType::Geometry).map(|(v, _)| v.clone());
        node.outputs.iter().map(|s| if s.ty == SocketType::Geometry { g.clone().unwrap_or_default() } else { own_value(s) }).collect()
    }

    /// Items fed into a zone input node (its inputs after Iterations for
    /// repeat zones; all of them for simulation zones), extend socket excluded.
    fn zone_initial(&self, run: &mut Run, ni: usize) -> Vec<SocketValue> {
        let node = &run.tree.nodes[ni];
        let skip = usize::from(node.idname == "GeometryNodeRepeatInput");
        (skip..node.inputs.len()).filter(|&i| node.inputs[i].ty != SocketType::Virtual).map(|i| self.input(run, ni, i)).collect()
    }

    /// Nodes whose value depends on node `zin` (the zone body and output).
    fn zone_nodes(tree: &NodeTree, zin: usize) -> Vec<usize> {
        (0..tree.nodes.len()).filter(|&n| n != zin && tree.depends_on(n, zin)).collect()
    }

    fn zone_items(&self, run: &mut Run, out: usize, skip: usize) -> Vec<SocketValue> {
        let node = &run.tree.nodes[out];
        (skip..node.inputs.len()).filter(|&i| node.inputs[i].ty != SocketType::Virtual).map(|i| self.input(run, out, i)).collect()
    }

    fn repeat(&self, run: &mut Run, out: usize) -> Vec<SocketValue> {
        let Some(zin) = paired_input(run.tree, out, "GeometryNodeRepeatInput") else { return Vec::new() };
        let iters = run.tree.nodes[zin].input("Iterations").map(|i| self.input(run, zin, i).val().i()).unwrap_or(1).clamp(0, 100_000);
        let mut state = self.zone_initial(run, zin);
        let body = Self::zone_nodes(run.tree, zin);
        for it in 0..iters {
            let ov: Vec<SocketValue> = std::iter::once(SocketValue::single(Val::Int(it))).chain(state.iter().cloned().map(freeze)).collect();
            run.overrides.insert(zin, ov);
            for n in &body {
                run.memo.remove(n);
            }
            state = self.zone_items(run, out, 0);
        }
        run.overrides.remove(&zin);
        for n in &body {
            run.memo.remove(n);
        }
        state
    }

    fn simulation(&self, run: &mut Run, out: usize) -> Vec<SocketValue> {
        let Some(zin) = paired_input(run.tree, out, "GeometryNodeSimulationInput") else { return Vec::new() };
        let initial: Vec<SocketValue> = self.zone_initial(run, zin).into_iter().map(freeze).collect();
        let scene = self.opts.scene;
        let start = scene.map_or(1, |s| s.frame_start);
        let fps = scene.map_or(24.0, |s| s.fps).max(1e-3);
        let frame = self.opts.frame.round() as i32;
        let owner = self.opts.self_object.and_then(|o| scene.and_then(|s| s.objects.get(o))).map_or(String::new(), |o| o.name.clone());
        let key = format!("{}|{owner}|{}|{}", scene.map_or("", |s| s.name.as_str()), run.path, run.tree.nodes[out].name);
        if frame < start {
            return initial;
        }
        let body = Self::zone_nodes(run.tree, zin);
        let step = |run: &mut Run, state: Vec<SocketValue>, dt: f32| -> Vec<SocketValue> {
            let ov: Vec<SocketValue> = std::iter::once(SocketValue::single(Val::Float(dt))).chain(state.iter().cloned()).collect();
            run.overrides.insert(zin, ov);
            for n in &body {
                run.memo.remove(n);
            }
            let skip = run.tree.nodes[out].input("Skip").map(|i| self.input(run, out, i).val().b()).unwrap_or(false);
            let next = if skip { state } else { self.zone_items(run, out, 1).into_iter().map(freeze).collect() };
            run.overrides.remove(&zin);
            for n in &body {
                run.memo.remove(n);
            }
            next
        };
        let cached = |f: i32| sim_store().lock().ok().and_then(|s| s.get(&key).and_then(|m| m.get(&f).cloned()));
        let store = |f: i32, v: &Vec<SocketValue>| {
            if let Ok(mut s) = sim_store().lock() {
                let m = s.entry(key.clone()).or_default();
                m.insert(f, v.clone());
                while m.len() > 2000 {
                    let first = *m.keys().next().unwrap();
                    m.remove(&first);
                }
            }
        };
        if frame == start {
            if let Ok(mut s) = sim_store().lock() {
                s.remove(&key);
            }
            let st = step(run, initial, 0.0);
            store(start, &st);
            return st;
        }
        if let Some(v) = cached(frame) {
            return v;
        }
        let last = sim_store().lock().ok().and_then(|s| s.get(&key).and_then(|m| m.range(start..frame).next_back().map(|(f, v)| (*f, v.clone()))));
        let (mut f0, mut state) = match last {
            Some(x) => x,
            None => {
                let st = step(run, initial, 0.0);
                store(start, &st);
                (start, st)
            }
        };
        while f0 < frame {
            state = step(run, state, 1.0 / fps);
            f0 += 1;
            store(f0, &state);
        }
        state
    }
}

/// Simulation and repeat state holds plain values: fields that do not
/// depend on geometry are folded to their value.
fn freeze(v: SocketValue) -> SocketValue {
    match v {
        SocketValue::Field(f) if !f.is_const() && !depends_on_context(&f) => SocketValue::single(eval_single(&f)),
        other => other,
    }
}

fn depends_on_context(f: &Field) -> bool {
    match &**f {
        FieldNode::Const(_) => false,
        FieldNode::Fn { op, .. } => op.inputs.iter().any(depends_on_context),
        _ => true,
    }
}

fn constant(node: &Node) -> Option<SocketValue> {
    let s = node.outputs.first()?;
    let v = match (node.idname.as_str(), node.props.first().map(|p| &p.1)) {
        ("ShaderNodeValue", _) => value_to_val(&s.value, SocketType::Float),
        ("FunctionNodeInputVector" | "GeometryNodeInputVector", Some(Prop::Vector(x))) => Val::Vector(Vec3::from_array(*x)),
        ("FunctionNodeInputInt", Some(Prop::Int(x))) => Val::Int(*x),
        ("FunctionNodeInputBool", Some(Prop::Bool(x))) => Val::Bool(*x),
        ("FunctionNodeInputColor", Some(Prop::Color(x))) => Val::Color(*x),
        ("FunctionNodeInputRotation", Some(Prop::Vector(x))) => Val::Rotation(crate::math::Quat::from_euler(*x, crate::math::EulerOrder::XYZ)),
        _ => value_to_val(&s.value, s.ty),
    };
    Some(SocketValue::single(convert_to(v, s.ty)))
}

/// Switch / Index Switch / Menu Switch on a single condition: picks one
/// input whole (geometry, strings, fields).
fn switch_single(ins: &Ins) -> Vec<SocketValue> {
    let node = ins.node;
    let pick = match node.idname.as_str() {
        "GeometryNodeSwitch" => {
            if ins.b("Switch") {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        "GeometryNodeIndexSwitch" => format!("Item_{}", ins.i("Index")),
        _ => {
            // Menu Switch: the Menu input names the chosen item.
            let m = ins.s("Menu");
            let m = if m.is_empty() { node.inputs.get(1).map(|s| s.name.clone()).unwrap_or_default() } else { m };
            node.inputs.iter().skip(1).find(|s| s.name == m || s.identifier == m).map(|s| s.identifier.clone()).unwrap_or_default()
        }
    };
    let v = match node.input(&pick) {
        Some(i) => ins.vals[i].clone(),
        None => SocketValue::None,
    };
    let mut out = vec![SocketValue::None; node.outputs.len()];
    if let Some(o) = out.first_mut() {
        *o = match v {
            SocketValue::None => node.outputs.first().map(own_value).unwrap_or_default(),
            v => v,
        };
    }
    out
}

/// Evaluate node group `group` with `inputs` (one per interface input,
/// missing ones take the interface defaults); returns the interface outputs.
pub fn evaluate_group(group: &NodeTree, inputs: Vec<SocketValue>, opts: &EvalOptions) -> Vec<SocketValue> {
    let ev = Evaluator { opts, groups: opts.groups() };
    let mut ins = inputs;
    for (k, s) in group.inputs.iter().enumerate().skip(ins.len()) {
        let _ = k;
        ins.push(own_value(&crate::nodes::Socket::new(&s.name, s.ty, if s.default == Value::None { edit::type_default(s.ty) } else { s.default.clone() })));
    }
    ev.eval_tree(group, ins, "", 0)
}

/// The Nodes modifier: evaluate group `group_name` with the mesh as the
/// first geometry input and `inputs` (by interface name) for the rest, then
/// realize instances and turn the result into a mesh (curves become wire
/// edges along their evaluated points, point clouds loose vertices).
pub fn run_modifier(mesh: &Mesh, scene: &Scene, group_name: &str, inputs: &[(String, Value)], owner: usize, frame: f32, depth: u32) -> Mesh {
    let Some(group) = scene.node_groups.iter().find(|g| g.name == group_name) else { return mesh.clone() };
    let opts = EvalOptions { scene: Some(scene), frame, self_object: Some(owner), depth };
    let mut ins = Vec::new();
    let mut first_geo = true;
    for s in &group.inputs {
        let v = if s.ty == SocketType::Geometry {
            if std::mem::replace(&mut first_geo, false) {
                SocketValue::Geometry(Geometry::from_mesh(mesh.clone()))
            } else {
                SocketValue::Geometry(Geometry::default())
            }
        } else {
            let val = inputs.iter().find(|(n, _)| *n == s.name).map(|x| x.1.clone()).unwrap_or_else(|| if s.default == Value::None { edit::type_default(s.ty) } else { s.default.clone() });
            own_value(&crate::nodes::Socket::new(&s.name, s.ty, val))
        };
        ins.push(v);
    }
    let outs = evaluate_group(group, ins, &opts);
    let Some(k) = group.outputs.iter().position(|s| s.ty == SocketType::Geometry) else { return mesh.clone() };
    let g = outs[k].geometry();
    geometry_to_mesh(&g, &|r| opts.resolve(r))
}

/// Realize a geometry set into one mesh for display and rendering.
pub fn geometry_to_mesh(g: &Geometry, resolve: &dyn Fn(&InstanceRef) -> Option<Geometry>) -> Mesh {
    let g = realize(g, resolve);
    let mut out = g.mesh.unwrap_or_default();
    if let Some(c) = &g.curves {
        let mut wire = Mesh::default();
        for ci in 0..c.curves_num() {
            let ev = c.evaluate(ci);
            let base = wire.positions.len() as u32;
            wire.positions.extend_from_slice(&ev.positions);
            let n = ev.positions.len() as u32;
            for i in 0..n.saturating_sub(1) {
                wire.loose_edges.push([base + i, base + i + 1]);
            }
            if ev.cyclic && n > 2 {
                wire.loose_edges.push([base + n - 1, base]);
            }
        }
        out = super::join_meshes(&out, &wire);
    }
    if let Some(p) = &g.points {
        let pts = Mesh { positions: p.positions.clone(), ..Default::default() };
        out = super::join_meshes(&out, &pts);
    }
    // Anonymous attributes (Capture Attribute) do not leave the tree.
    out.attributes.list.retain(|a| !a.name.starts_with('.'));
    out.attributes.retain_valid({
        let t = super::MeshTopo::new(&out);
        let m = out.clone();
        move |d| t.size(&m, d)
    });
    out
}

/// The interface inputs a Nodes modifier shows: every non-geometry input of
/// the group, with its default.
pub fn modifier_interface(group: &NodeTree) -> Vec<(String, SocketType, Value)> {
    let mut seen = HashSet::new();
    group
        .inputs
        .iter()
        .filter(|s| s.ty != SocketType::Geometry && seen.insert(s.name.clone()))
        .map(|s| (s.name.clone(), s.ty, if s.default == Value::None { edit::type_default(s.ty) } else { s.default.clone() }))
        .collect()
}

/// Unique names for anonymous attributes (Capture Attribute).
pub(crate) fn anonymous_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(".a_{}", N.fetch_add(1, Ordering::Relaxed))
}

pub(crate) fn domain_prop(node: &Node, key: &str) -> Domain {
    Domain::from_name(node.enum_prop(key)).unwrap_or(Domain::Point)
}
