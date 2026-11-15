// SPDX-License-Identifier: GPL-3.0-only
//! Node tree to [`Program`] compiler (see the module docs of `shader`).
use super::color::Blend;
use super::ops::{MapInterp, MappingType, MathOp, VecOp};
use super::tex::{NoiseType, VoronoiFeature, VoronoiMetric, WaveType};
use super::vm::{self, Implicit, Op, PRINCIPLED_INS};
use super::{ClosureKind, Instr, Program, Reg, ShadeCtx, ShaderOp, V4};
use crate::nodes::{new_node, Node, NodeTree, Prop, SocketType, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Float,
    Vector,
    Color,
}

#[derive(Clone, Copy, Debug)]
enum Src {
    Data(Reg, Kind),
    Shader(Option<u32>),
    None,
}

fn kind_of(t: SocketType) -> Option<Kind> {
    match t {
        SocketType::Float | SocketType::Int | SocketType::Bool => Some(Kind::Float),
        SocketType::Vector | SocketType::Rotation => Some(Kind::Vector),
        SocketType::Color => Some(Kind::Color),
        _ => None,
    }
}

fn value_v4(v: &Value, k: Kind) -> V4 {
    match k {
        Kind::Float => {
            let f = v.as_float();
            [f, f, f, 1.0]
        }
        Kind::Vector => {
            let a = v.as_vector();
            [a[0], a[1], a[2], 0.0]
        }
        Kind::Color => v.as_color(),
    }
}

struct Frame<'a> {
    tree: &'a NodeTree,
    /// (parent frame, group node in it).
    parent: Option<(usize, usize)>,
}

struct Compiler<'a> {
    groups: &'a [NodeTree],
    world: bool,
    prog: Program,
    konst: Vec<bool>,
    const_map: HashMap<[u32; 4], Reg>,
    frames: Vec<Frame<'a>>,
    frame_map: HashMap<(usize, usize), usize>,
    memo: HashMap<(usize, usize, u8), Vec<Src>>,
    busy: HashSet<(usize, usize, u8)>,
    implicit: HashMap<(u8, u8), Reg>,
    to_disp: bool,
    canon: HashMap<String, Option<Node>>,
}

/// Compile a material tree. Returns `None` when the tree has no usable
/// output (the renderer then falls back to the legacy fields).
pub(crate) fn compile_material(tree: &NodeTree, groups: &[NodeTree]) -> Option<Program> {
    let out = active_output(tree, "ShaderNodeOutputMaterial")?;
    let mut c = Compiler::new(tree, groups, false);
    let node = &tree.nodes[out];
    if let Some(si) = node.input("Displacement") {
        if c.linked(0, out, si) {
            c.to_disp = true;
            let d: [Reg; 3] = core::array::from_fn(|v| c.get(0, out, "Displacement", v as u8, Kind::Vector));
            c.prog.disp_out = Some(d);
            c.to_disp = false;
            c.memo.clear();
            c.implicit.clear();
        }
    }
    c.prog.surface = node.input("Surface").and_then(|s| c.shader(0, out, s, 0));
    c.prog.volume = node.input("Volume").and_then(|s| c.shader(0, out, s, 0));
    Some(c.finish())
}

/// Compile a world tree.
pub(crate) fn compile_world(tree: &NodeTree, groups: &[NodeTree]) -> Option<Program> {
    let out = active_output(tree, "ShaderNodeOutputWorld")?;
    let mut c = Compiler::new(tree, groups, true);
    let node = &tree.nodes[out];
    c.prog.surface = node.input("Surface").and_then(|s| c.shader(0, out, s, 0));
    c.prog.volume = node.input("Volume").and_then(|s| c.shader(0, out, s, 0));
    Some(c.finish())
}

/// The output node Cycles renders: target All or Cycles, the active one first.
fn active_output(tree: &NodeTree, idname: &str) -> Option<usize> {
    let ok = |n: &Node| n.idname == idname && matches!(n.enum_prop("target"), "" | "ALL" | "CYCLES");
    let mut first = None;
    for (i, n) in tree.nodes.iter().enumerate() {
        if ok(n) {
            if n.bool_prop("is_active_output") {
                return Some(i);
            }
            first.get_or_insert(i);
        }
    }
    first
}

fn prop_i(n: &Node, k: &str, d: i32) -> i32 {
    match n.prop(k) {
        Some(Prop::Int(i)) => *i,
        Some(Prop::Float(f)) => *f as i32,
        _ => d,
    }
}

fn prop_f(n: &Node, k: &str, d: f32) -> f32 {
    match n.prop(k) {
        Some(Prop::Float(f)) => *f,
        Some(Prop::Int(i)) => *i as f32,
        _ => d,
    }
}

fn prop_v(n: &Node, k: &str, d: [f32; 3]) -> [f32; 3] {
    match n.prop(k) {
        Some(Prop::Vector(v)) => *v,
        _ => d,
    }
}

fn prop_id(n: &Node, k: &str) -> Option<usize> {
    match n.prop(k) {
        Some(Prop::Id(i)) => *i,
        _ => None,
    }
}

fn dims(s: &str) -> u8 {
    match s {
        "1D" => 1,
        "2D" => 2,
        "4D" => 4,
        _ => 3,
    }
}

impl<'a> Compiler<'a> {
    fn new(tree: &'a NodeTree, groups: &'a [NodeTree], world: bool) -> Compiler<'a> {
        Compiler {
            groups,
            world,
            prog: Program::default(),
            konst: Vec::new(),
            const_map: HashMap::new(),
            frames: vec![Frame { tree, parent: None }],
            frame_map: HashMap::new(),
            memo: HashMap::new(),
            busy: HashSet::new(),
            implicit: HashMap::new(),
            to_disp: false,
            canon: HashMap::new(),
        }
    }

    fn finish(mut self) -> Program {
        self.prog.info = self.info();
        self.prog
    }

    fn info(&self) -> super::ProgramInfo {
        let mut info = super::ProgramInfo::default();
        let mut stack: Vec<u32> = self.prog.surface.into_iter().collect();
        let mut seen = HashSet::new();
        let zero = |r: Reg| self.konst.get(r as usize).copied().unwrap_or(false) && self.prog.init[r as usize][..3].iter().all(|x| *x == 0.0);
        let is_one = |r: Reg| self.konst.get(r as usize).copied().unwrap_or(false) && self.prog.init[r as usize][0] >= 1.0;
        while let Some(s) = stack.pop() {
            if !seen.insert(s) {
                continue;
            }
            match &self.prog.shaders[s as usize] {
                ShaderOp::Mix { a, b, .. } | ShaderOp::Add { a, b } => stack.extend(a.iter().chain(b.iter())),
                ShaderOp::Closure { kind, ins } => match kind {
                    ClosureKind::Emission => info.emission |= !(zero(ins[0]) || zero(ins[1])),
                    ClosureKind::Principled { .. } => {
                        info.emission |= !(zero(ins[25]) || zero(ins[26]));
                        info.transparent |= !is_one(ins[4]);
                    }
                    ClosureKind::Transparent => info.transparent = true,
                    _ => {}
                },
            }
        }
        info.volume = self.prog.volume.is_some();
        if info.volume && self.prog.surface.is_none() {
            info.transparent = true;
        }
        info.uv_tangent = self.prog.surf.iter().chain(&self.prog.disp).any(|i| matches!(i.op, Op::NormalMap { space: 0 } | Op::VecDisplacement { space: 0 } | Op::TangentNode { axis: None }));
        info
    }

    fn tree(&self, fr: usize) -> &'a NodeTree {
        self.frames[fr].tree
    }

    fn alloc(&mut self, n: usize) -> Reg {
        let r = self.prog.init.len() as Reg;
        for _ in 0..n.max(1) {
            self.prog.init.push([0.0; 4]);
            self.konst.push(false);
        }
        r
    }

    fn konst(&mut self, v: V4) -> Reg {
        let key = v.map(f32::to_bits);
        if let Some(&r) = self.const_map.get(&key) {
            return r;
        }
        let r = self.alloc(1);
        self.prog.init[r as usize] = v;
        self.konst[r as usize] = true;
        self.const_map.insert(key, r);
        r
    }

    /// Add an instruction writing `n_out` registers; fold it when pure with
    /// constant inputs.
    fn emit(&mut self, op: Op, ins: Vec<Reg>, n_out: usize, variant: u8) -> Reg {
        let out = self.alloc(n_out);
        let ins = Instr { op, ins, out, variant };
        if ins.op.pure() && ins.ins.iter().all(|&r| self.konst[r as usize]) {
            vm::exec(&ins, &ShadeCtx::default(), &mut self.prog.init);
            for k in 0..n_out.max(1) {
                self.konst[out as usize + k] = true;
            }
        } else if self.to_disp {
            self.prog.disp.push(ins);
        } else {
            self.prog.surf.push(ins);
        }
        out
    }

    fn implicit(&mut self, k: Implicit, v: u8) -> Reg {
        let key = (k as u8, v);
        if let Some(&r) = self.implicit.get(&key) {
            return r;
        }
        let r = self.emit(Op::Implicit(k), vec![], 1, v);
        self.implicit.insert(key, r);
        r
    }

    fn convert(&mut self, s: Src, k: Kind, v: u8) -> Reg {
        match s {
            Src::Data(r, from) => match (from, k) {
                (a, b) if a == b => r,
                (Kind::Color, Kind::Vector) => r,
                (Kind::Float, _) => self.emit(Op::FloatToVec, vec![r], 1, v),
                (Kind::Color, Kind::Float) => self.emit(Op::ColorToFloat, vec![r], 1, v),
                (Kind::Vector, Kind::Float) => self.emit(Op::VecToFloat, vec![r], 1, v),
                (Kind::Vector, Kind::Color) => self.emit(Op::ToColor, vec![r], 1, v),
                _ => r,
            },
            _ => self.konst(if k == Kind::Color { [0.0, 0.0, 0.0, 1.0] } else { [0.0; 4] }),
        }
    }

    fn linked(&self, fr: usize, node: usize, sock: usize) -> bool {
        self.tree(fr).links.iter().any(|l| l.to_node == node && l.to_socket == sock && !l.muted)
    }

    /// What feeds input `sock` of `node` (`Src::None` when unlinked).
    fn input(&mut self, fr: usize, node: usize, sock: usize, v: u8) -> Src {
        let link = self.tree(fr).links.iter().find(|l| l.to_node == node && l.to_socket == sock && !l.muted).copied();
        match link {
            Some(l) if l.from_node < self.tree(fr).nodes.len() => self.output(fr, l.from_node, l.from_socket, v),
            _ => Src::None,
        }
    }

    /// The input's link or its own value.
    fn input_or_value(&mut self, fr: usize, node: usize, sock: usize, v: u8) -> Src {
        match self.input(fr, node, sock, v) {
            Src::None => {
                let Some(s) = self.tree(fr).nodes[node].inputs.get(sock) else { return Src::None };
                if s.ty == SocketType::Shader {
                    return Src::Shader(None);
                }
                match kind_of(s.ty) {
                    Some(k) => {
                        let val = value_v4(&s.value, k);
                        Src::Data(self.konst(val), k)
                    }
                    None => Src::None,
                }
            }
            s => s,
        }
    }

    fn output(&mut self, fr: usize, node: usize, sock: usize, v: u8) -> Src {
        let tree = self.tree(fr);
        let n = &tree.nodes[node];
        let Some(os) = n.outputs.get(sock) else { return Src::None };
        if n.idname == "NodeReroute" {
            return self.input_or_value(fr, node, 0, v);
        }
        if n.mute {
            // Blender's internal links: the first input of the same type, else
            // the first data input for a data output.
            let ty = os.ty;
            let pick = n.inputs.iter().position(|s| s.enabled && s.ty == ty).or_else(|| {
                if ty == SocketType::Shader {
                    None
                } else {
                    n.inputs.iter().position(|s| s.enabled && kind_of(s.ty).is_some() && kind_of(ty).is_some())
                }
            });
            return match pick {
                Some(i) => self.input_or_value(fr, node, i, v),
                None => Src::None,
            };
        }
        match n.idname.as_str() {
            "NodeGroupInput" => {
                return match self.frames[fr].parent {
                    Some((pf, gn)) => self.input_or_value(pf, gn, sock, v),
                    None => match tree.inputs.get(sock) {
                        Some(i) => match kind_of(i.ty) {
                            Some(k) => {
                                let val = value_v4(&i.default, k);
                                Src::Data(self.konst(val), k)
                            }
                            None => Src::Shader(None),
                        },
                        None => Src::None,
                    },
                };
            }
            "ShaderNodeGroup" => {
                let Some(g) = prop_id(n, "node_tree").and_then(|g| self.groups.get(g)) else { return Src::None };
                if self.frames.len() > 256 || self.depth(fr) > 32 {
                    return Src::None;
                }
                let f = match self.frame_map.get(&(fr, node)) {
                    Some(&f) => f,
                    None => {
                        self.frames.push(Frame { tree: g, parent: Some((fr, node)) });
                        let f = self.frames.len() - 1;
                        self.frame_map.insert((fr, node), f);
                        f
                    }
                };
                let Some(out) = active_group_output(g) else { return Src::None };
                return self.input_or_value(f, out, sock, v);
            }
            _ => {}
        }
        let key = (fr, node, v);
        if let Some(o) = self.memo.get(&key) {
            return o.get(sock).copied().unwrap_or(Src::None);
        }
        if !self.busy.insert(key) {
            return Src::None; // a cycle
        }
        let outs = self.node(fr, node, v);
        self.busy.remove(&key);
        // Map the canonical outputs onto this node's sockets by identifier.
        let canon_ids: Vec<String> = match self.canon_node(&n.idname) {
            Some(c) => c.outputs.iter().map(|s| s.identifier.clone()).collect(),
            None => Vec::new(),
        };
        let mapped: Vec<Src> = n
            .outputs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let k = canon_ids.iter().position(|id| *id == s.identifier).unwrap_or(i);
                outs.get(k).copied().unwrap_or(Src::None)
            })
            .collect();
        let r = mapped.get(sock).copied().unwrap_or(Src::None);
        self.memo.insert(key, mapped);
        r
    }

    fn depth(&self, mut fr: usize) -> usize {
        let mut d = 0;
        while let Some((p, _)) = self.frames[fr].parent {
            fr = p;
            d += 1;
        }
        d
    }

    fn canon_node(&mut self, idname: &str) -> Option<&Node> {
        self.canon.entry(idname.to_string()).or_insert_with(|| new_node(idname)).as_ref()
    }

    /// The socket index of input `id` on `node` (identifier, then name).
    fn sock(&self, fr: usize, node: usize, id: &str) -> Option<usize> {
        self.tree(fr).nodes[node].input(id)
    }

    /// Input `id` as a register of kind `k`: its link, its value, or the
    /// canonical default.
    fn get(&mut self, fr: usize, node: usize, id: &str, v: u8, k: Kind) -> Reg {
        match self.sock(fr, node, id) {
            Some(s) => {
                let src = self.input_or_value(fr, node, s, v);
                self.convert(src, k, v)
            }
            None => {
                let idname = self.tree(fr).nodes[node].idname.clone();
                let val = self.canon_node(&idname).and_then(|c| c.input(id).map(|i| c.inputs[i].value.clone())).unwrap_or(Value::None);
                let val = value_v4(&val, k);
                self.konst(val)
            }
        }
    }

    /// Like `get`, with an implicit context value when unlinked.
    fn get_or(&mut self, fr: usize, node: usize, id: &str, v: u8, k: Kind, imp: Implicit) -> Reg {
        if let Some(s) = self.sock(fr, node, id) {
            let src = self.input(fr, node, s, v);
            if !matches!(src, Src::None) {
                return self.convert(src, k, v);
            }
        }
        self.implicit(imp, v)
    }

    fn f(&mut self, fr: usize, node: usize, id: &str, v: u8) -> Reg {
        self.get(fr, node, id, v, Kind::Float)
    }

    fn vec(&mut self, fr: usize, node: usize, id: &str, v: u8) -> Reg {
        self.get(fr, node, id, v, Kind::Vector)
    }

    fn col(&mut self, fr: usize, node: usize, id: &str, v: u8) -> Reg {
        self.get(fr, node, id, v, Kind::Color)
    }

    fn normal(&mut self, fr: usize, node: usize, id: &str, v: u8) -> Reg {
        self.get_or(fr, node, id, v, Kind::Vector, Implicit::Normal)
    }

    /// Texture vector input: generated coordinates when unlinked.
    fn texco(&mut self, fr: usize, node: usize, v: u8, imp: Implicit) -> Reg {
        self.get_or(fr, node, "Vector", v, Kind::Vector, imp)
    }

    /// A shader input as a closure-tree index.
    fn shader(&mut self, fr: usize, node: usize, sock: usize, v: u8) -> Option<u32> {
        match self.input(fr, node, sock, v) {
            Src::Shader(s) => s,
            // A colour linked into a shader socket emits (as in Blender).
            Src::Data(r, k) => {
                let c = self.convert(Src::Data(r, k), Kind::Color, v);
                let one = self.konst([1.0; 4]);
                let kind = if self.world { ClosureKind::Background } else { ClosureKind::Emission };
                Some(self.push_shader(ShaderOp::Closure { kind, ins: vec![c, one] }))
            }
            Src::None => None,
        }
    }

    fn shader_id(&mut self, fr: usize, node: usize, id: &str, v: u8) -> Option<u32> {
        let s = self.sock(fr, node, id)?;
        self.shader(fr, node, s, v)
    }

    fn push_shader(&mut self, s: ShaderOp) -> u32 {
        self.prog.shaders.push(s);
        self.prog.shaders.len() as u32 - 1
    }

    fn closure(&mut self, kind: ClosureKind, ins: Vec<Reg>) -> Vec<Src> {
        let s = self.push_shader(ShaderOp::Closure { kind, ins });
        vec![Src::Shader(Some(s))]
    }

    fn data(out: Reg, kinds: &[Kind]) -> Vec<Src> {
        kinds.iter().enumerate().map(|(i, k)| Src::Data(out + i as Reg, *k)).collect()
    }

    /// Compile one node; outputs in the registry's socket order.
    fn node(&mut self, fr: usize, ni: usize, v: u8) -> Vec<Src> {
        use Kind::{Color as C, Float as F, Vector as V};
        let n: &'a Node = &self.tree(fr).nodes[ni];
        let ms = |n: &Node| matches!(n.enum_prop("distribution"), "MULTI_GGX" | "");
        match n.idname.as_str() {
            // ---- Shaders
            "ShaderNodeBsdfPrincipled" => {
                let ins = PRINCIPLED_INS
                    .iter()
                    .map(|id| match *id {
                        "Normal" | "Coat Normal" => self.normal(fr, ni, id, v),
                        "Tangent" => self.get_or(fr, ni, id, v, V, Implicit::Tangent),
                        "Base Color" | "Specular Tint" | "Coat Tint" | "Sheen Tint" | "Emission Color" => self.col(fr, ni, id, v),
                        "Subsurface Radius" => self.vec(fr, ni, id, v),
                        _ => self.f(fr, ni, id, v),
                    })
                    .collect();
                self.closure(ClosureKind::Principled { ms: ms(n) }, ins)
            }
            "ShaderNodeBsdfDiffuse" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Roughness", v), self.normal(fr, ni, "Normal", v)];
                self.closure(ClosureKind::Diffuse, ins)
            }
            "ShaderNodeEmission" | "ShaderNodeBackground" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Strength", v)];
                let k = if n.idname == "ShaderNodeBackground" || self.world { ClosureKind::Background } else { ClosureKind::Emission };
                self.closure(k, ins)
            }
            "ShaderNodeBsdfGlass" | "ShaderNodeBsdfRefraction" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Roughness", v), self.f(fr, ni, "IOR", v), self.normal(fr, ni, "Normal", v)];
                let k = if n.idname == "ShaderNodeBsdfGlass" { ClosureKind::Glass { ms: ms(n) } } else { ClosureKind::Refraction { ms: ms(n) } };
                self.closure(k, ins)
            }
            "ShaderNodeBsdfAnisotropic" => {
                let ins = vec![
                    self.col(fr, ni, "Color", v),
                    self.f(fr, ni, "Roughness", v),
                    self.f(fr, ni, "Anisotropy", v),
                    self.f(fr, ni, "Rotation", v),
                    self.normal(fr, ni, "Normal", v),
                    self.get_or(fr, ni, "Tangent", v, V, Implicit::Tangent),
                ];
                self.closure(ClosureKind::Glossy { ms: ms(n) }, ins)
            }
            "ShaderNodeBsdfGlossy" => {
                let zero = self.konst([0.0; 4]);
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Roughness", v), zero, zero, self.normal(fr, ni, "Normal", v), zero];
                self.closure(ClosureKind::Glossy { ms: n.enum_prop("distribution") == "MULTI_GGX" }, ins)
            }
            "ShaderNodeBsdfSheen" | "ShaderNodeBsdfVelvet" => {
                let r = if n.idname == "ShaderNodeBsdfVelvet" { "Sigma" } else { "Roughness" };
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, r, v), self.normal(fr, ni, "Normal", v)];
                self.closure(ClosureKind::Sheen, ins)
            }
            "ShaderNodeBsdfTranslucent" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.normal(fr, ni, "Normal", v)];
                self.closure(ClosureKind::Translucent, ins)
            }
            "ShaderNodeBsdfTransparent" => {
                let ins = vec![self.col(fr, ni, "Color", v)];
                self.closure(ClosureKind::Transparent, ins)
            }
            "ShaderNodeHoldout" => self.closure(ClosureKind::Holdout, vec![]),
            "ShaderNodeBsdfToon" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Size", v), self.f(fr, ni, "Smooth", v), self.normal(fr, ni, "Normal", v)];
                self.closure(ClosureKind::Toon { glossy: n.enum_prop("component") == "GLOSSY" }, ins)
            }
            "ShaderNodeSubsurfaceScattering" => {
                let mut ins: Vec<Reg> = ["Color", "Scale", "Radius", "IOR", "Roughness", "Anisotropy"].iter().map(|id| if *id == "Color" { self.col(fr, ni, id, v) } else if *id == "Radius" { self.vec(fr, ni, id, v) } else { self.f(fr, ni, id, v) }).collect();
                ins.push(self.normal(fr, ni, "Normal", v));
                self.closure(ClosureKind::Subsurface, ins)
            }
            "ShaderNodeBsdfHair" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Offset", v), self.f(fr, ni, "RoughnessU", v), self.f(fr, ni, "RoughnessV", v), self.get_or(fr, ni, "Tangent", v, V, Implicit::Tangent)];
                self.closure(ClosureKind::Hair, ins)
            }
            "ShaderNodeBsdfHairPrincipled" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Roughness", v)];
                self.closure(ClosureKind::HairPrincipled, ins)
            }
            "ShaderNodeVolumePrincipled" => {
                let ins = vec![
                    self.col(fr, ni, "Color", v),
                    self.f(fr, ni, "Density", v),
                    self.f(fr, ni, "Anisotropy", v),
                    self.col(fr, ni, "Absorption Color", v),
                    self.f(fr, ni, "Emission Strength", v),
                    self.col(fr, ni, "Emission Color", v),
                    self.f(fr, ni, "Blackbody Intensity", v),
                    self.col(fr, ni, "Blackbody Tint", v),
                    self.f(fr, ni, "Temperature", v),
                ];
                self.closure(ClosureKind::VolumePrincipled, ins)
            }
            "ShaderNodeVolumeAbsorption" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Density", v)];
                self.closure(ClosureKind::VolumeAbsorption, ins)
            }
            "ShaderNodeVolumeScatter" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Density", v), self.f(fr, ni, "Anisotropy", v)];
                self.closure(ClosureKind::VolumeScatter, ins)
            }
            "ShaderNodeMixShader" => {
                let fac = self.f(fr, ni, "Fac", v);
                let a = self.shader_id(fr, ni, "Shader", v);
                let b = self.shader_id(fr, ni, "Shader_001", v);
                vec![Src::Shader(Some(self.push_shader(ShaderOp::Mix { fac, a, b })))]
            }
            "ShaderNodeAddShader" => {
                let a = self.shader_id(fr, ni, "Shader", v);
                let b = self.shader_id(fr, ni, "Shader_001", v);
                vec![Src::Shader(Some(self.push_shader(ShaderOp::Add { a, b })))]
            }
            // ---- Inputs
            "ShaderNodeValue" => vec![Src::Data(self.konst(value_v4(&n.outputs.first().map_or(Value::Float(0.0), |s| s.value.clone()), F)), F)],
            "ShaderNodeRGB" => vec![Src::Data(self.konst(value_v4(&n.outputs.first().map_or(Value::Color([0.5, 0.5, 0.5, 1.0]), |s| s.value.clone()), C)), C)],
            "ShaderNodeFresnel" => {
                let ins = vec![self.f(fr, ni, "IOR", v), self.normal(fr, ni, "Normal", v)];
                Self::data(self.emit(Op::Fresnel, ins, 1, v), &[F])
            }
            "ShaderNodeLayerWeight" => {
                let ins = vec![self.f(fr, ni, "Blend", v), self.normal(fr, ni, "Normal", v)];
                Self::data(self.emit(Op::LayerWeight, ins, 2, v), &[F, F])
            }
            "ShaderNodeLightPath" => Self::data(self.emit(Op::LightPath, vec![], 14, v), &[F; 14]),
            "ShaderNodeNewGeometry" => Self::data(self.emit(Op::Geometry, vec![], 9, v), &[V, V, V, V, V, V, F, F, F]),
            "ShaderNodeTexCoord" => Self::data(self.emit(Op::TexCoord { object: prop_id(n, "object") }, vec![], 7, v), &[V; 7]),
            "ShaderNodeUVMap" => vec![Src::Data(self.implicit(Implicit::Uv, v), V)],
            "ShaderNodeObjectInfo" => Self::data(self.emit(Op::ObjectInfo, vec![], 6, v), &[V, C, F, F, F, F]),
            "ShaderNodeAttribute" => {
                let name = match n.prop("attribute_name") {
                    Some(Prop::String(s)) => s.to_ascii_lowercase(),
                    _ => String::new(),
                };
                let uv = name.contains("uv");
                Self::data(self.emit(Op::Attribute { uv }, vec![], 4, v), &[C, V, F, F])
            }
            "ShaderNodeTangent" => {
                let axis = if n.enum_prop("direction_type") == "UV_MAP" {
                    None
                } else {
                    Some(match n.enum_prop("axis") {
                        "X" => 0,
                        "Y" => 1,
                        _ => 2,
                    })
                };
                Self::data(self.emit(Op::TangentNode { axis }, vec![], 1, v), &[V])
            }
            "ShaderNodeCameraData" => Self::data(self.emit(Op::CameraData, vec![], 3, v), &[V, F, F]),
            "ShaderNodeWireframe" => {
                let ins = vec![self.f(fr, ni, "Size", v)];
                Self::data(self.emit(Op::Wireframe { pixel: n.bool_prop("use_pixel_size") }, ins, 1, v), &[F])
            }
            "ShaderNodeAmbientOcclusion" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Distance", v), self.normal(fr, ni, "Normal", v)];
                Self::data(self.emit(Op::Ao { samples: prop_i(n, "samples", 16).max(1) as u32, inside: n.bool_prop("inside") }, ins, 2, v), &[C, F])
            }
            "ShaderNodeBevel" => vec![Src::Data(self.normal(fr, ni, "Normal", v), V)],
            "ShaderNodeVertexColor" => vec![Src::Data(self.konst([1.0; 4]), C), Src::Data(self.konst([1.0; 4]), F)],
            "ShaderNodeHairInfo" | "ShaderNodeParticleInfo" | "ShaderNodePointInfo" | "ShaderNodeVolumeInfo" | "ShaderNodeShaderToRGB" => {
                let z = self.konst([0.0; 4]);
                n.outputs.iter().map(|s| Src::Data(z, kind_of(s.ty).unwrap_or(F))).collect()
            }
            "ShaderNodeLightFalloff" => {
                let ins = vec![self.f(fr, ni, "Strength", v), self.f(fr, ni, "Smooth", v)];
                Self::data(self.emit(Op::LightFalloff, ins, 3, v), &[F, F, F])
            }
            // ---- Textures
            "ShaderNodeTexImage" => {
                let ins = vec![self.texco(fr, ni, v, Implicit::Uv)];
                let projection = match n.enum_prop("projection") {
                    "BOX" => 1,
                    "SPHERE" => 2,
                    "TUBE" => 3,
                    _ => 0,
                };
                let op = Op::Image { image: prop_id(n, "image"), interp: vm::image_interp(n.enum_prop("interpolation")), ext: vm::image_ext(n.enum_prop("extension")), projection, blend: prop_f(n, "projection_blend", 0.0).clamp(0.0, 1.0) };
                Self::data(self.emit(op, ins, 2, v), &[C, F])
            }
            "ShaderNodeTexEnvironment" => {
                let ins = vec![self.texco(fr, ni, v, Implicit::Generated)];
                let op = Op::Environment { image: prop_id(n, "image"), interp: vm::image_interp(n.enum_prop("interpolation")), mirror_ball: n.enum_prop("projection") == "MIRROR_BALL" };
                Self::data(self.emit(op, ins, 1, v), &[C])
            }
            "ShaderNodeTexSky" => {
                let ins = vec![self.texco(fr, ni, v, Implicit::Generated)];
                let sky = match n.enum_prop("sky_type") {
                    "PREETHAM" | "HOSEK_WILKIE" => super::sky::Sky::Preetham(super::sky::Preetham::new(crate::math::Vec3::from_array(prop_v(n, "sun_direction", [0.0, 0.0, 1.0])), prop_f(n, "turbidity", 2.2))),
                    _ => super::sky::Sky::Nishita(Box::new(super::sky::Nishita::new(
                        prop_f(n, "sun_elevation", 0.2618),
                        prop_f(n, "sun_rotation", 0.0),
                        prop_f(n, "sun_size", 0.009_512),
                        prop_f(n, "sun_intensity", 1.0),
                        !matches!(n.prop("sun_disc"), Some(Prop::Bool(false))),
                        prop_f(n, "altitude", 0.0),
                        prop_f(n, "air_density", 1.0),
                        prop_f(n, "dust_density", 1.0),
                        prop_f(n, "ozone_density", 1.0),
                    ))),
                };
                Self::data(self.emit(Op::Sky(Arc::new(sky)), ins, 1, v), &[C])
            }
            "ShaderNodeTexNoise" | "ShaderNodeTexMusgrave" => {
                let musgrave = n.idname == "ShaderNodeTexMusgrave";
                let co = self.texco(fr, ni, v, Implicit::Generated);
                let mut ins = vec![co];
                let ids: &[&str] = if musgrave { &["W", "Scale", "Detail", "Dimension", "Lacunarity", "Offset", "Gain"] } else { &["W", "Scale", "Detail", "Roughness", "Lacunarity", "Offset", "Gain", "Distortion"] };
                for id in ids {
                    ins.push(self.f(fr, ni, id, v));
                }
                if musgrave {
                    let op = Op::Musgrave { dims: dims(n.enum_prop("musgrave_dimensions")), ty: NoiseType::from_name(n.enum_prop("musgrave_type")) };
                    Self::data(self.emit(op, ins, 1, v), &[F])
                } else {
                    let op = Op::Noise { dims: dims(n.enum_prop("noise_dimensions")), ty: NoiseType::from_name(n.enum_prop("noise_type")), normalize: !matches!(n.prop("normalize"), Some(Prop::Bool(false))) };
                    Self::data(self.emit(op, ins, 2, v), &[F, C])
                }
            }
            "ShaderNodeTexVoronoi" => {
                let mut ins = vec![self.texco(fr, ni, v, Implicit::Generated)];
                for id in ["W", "Scale", "Detail", "Roughness", "Lacunarity", "Smoothness", "Exponent", "Randomness"] {
                    ins.push(self.f(fr, ni, id, v));
                }
                let feature = match n.enum_prop("feature") {
                    "F2" => VoronoiFeature::F2,
                    "SMOOTH_F1" => VoronoiFeature::SmoothF1,
                    "DISTANCE_TO_EDGE" => VoronoiFeature::DistanceToEdge,
                    "N_SPHERE_RADIUS" => VoronoiFeature::NSphereRadius,
                    _ => VoronoiFeature::F1,
                };
                let metric = match n.enum_prop("distance") {
                    "MANHATTAN" => VoronoiMetric::Manhattan,
                    "CHEBYCHEV" => VoronoiMetric::Chebychev,
                    "MINKOWSKI" => VoronoiMetric::Minkowski,
                    _ => VoronoiMetric::Euclidean,
                };
                let op = Op::Voronoi { dims: dims(n.enum_prop("voronoi_dimensions")), feature, metric, normalize: n.bool_prop("normalize") };
                Self::data(self.emit(op, ins, 5, v), &[F, C, V, F, F])
            }
            "ShaderNodeTexWave" => {
                let mut ins = vec![self.texco(fr, ni, v, Implicit::Generated)];
                for id in ["Scale", "Distortion", "Detail", "Detail Scale", "Detail Roughness", "Phase Offset"] {
                    ins.push(self.f(fr, ni, id, v));
                }
                let rings = n.enum_prop("wave_type") == "RINGS";
                let direction = if rings {
                    match n.enum_prop("rings_direction") {
                        "Y" => 1,
                        "Z" => 2,
                        "SPHERICAL" => 3,
                        _ => 0,
                    }
                } else {
                    match n.enum_prop("bands_direction") {
                        "Y" => 1,
                        "Z" => 2,
                        "DIAGONAL" => 3,
                        _ => 0,
                    }
                };
                let profile = match n.enum_prop("wave_profile") {
                    "SAW" => 1,
                    "TRI" => 2,
                    _ => 0,
                };
                let op = Op::Wave { ty: if rings { WaveType::Rings } else { WaveType::Bands }, direction, profile };
                Self::data(self.emit(op, ins, 2, v), &[C, F])
            }
            "ShaderNodeTexMagic" => {
                let ins = vec![self.texco(fr, ni, v, Implicit::Generated), self.f(fr, ni, "Scale", v), self.f(fr, ni, "Distortion", v)];
                Self::data(self.emit(Op::Magic { depth: prop_i(n, "turbulence_depth", 2) }, ins, 2, v), &[C, F])
            }
            "ShaderNodeTexBrick" => {
                let mut ins = vec![self.texco(fr, ni, v, Implicit::Generated)];
                for id in ["Color1", "Color2", "Mortar"] {
                    ins.push(self.col(fr, ni, id, v));
                }
                for id in ["Scale", "Mortar Size", "Mortar Smooth", "Bias", "Brick Width", "Row Height"] {
                    ins.push(self.f(fr, ni, id, v));
                }
                let op = Op::Brick { offset: prop_f(n, "offset", 0.5), offset_frequency: prop_i(n, "offset_frequency", 2), squash: prop_f(n, "squash", 1.0), squash_frequency: prop_i(n, "squash_frequency", 2) };
                Self::data(self.emit(op, ins, 2, v), &[C, F])
            }
            "ShaderNodeTexChecker" => {
                let ins = vec![self.texco(fr, ni, v, Implicit::Generated), self.col(fr, ni, "Color1", v), self.col(fr, ni, "Color2", v), self.f(fr, ni, "Scale", v)];
                Self::data(self.emit(Op::Checker, ins, 2, v), &[C, F])
            }
            "ShaderNodeTexGradient" => {
                let ty = match n.enum_prop("gradient_type") {
                    "QUADRATIC" => 1,
                    "EASING" => 2,
                    "DIAGONAL" => 3,
                    "SPHERICAL" => 4,
                    "QUADRATIC_SPHERE" => 5,
                    "RADIAL" => 6,
                    _ => 0,
                };
                let ins = vec![self.texco(fr, ni, v, Implicit::Generated)];
                Self::data(self.emit(Op::Gradient(ty), ins, 2, v), &[C, F])
            }
            "ShaderNodeTexWhiteNoise" => {
                let ins = vec![self.texco(fr, ni, v, Implicit::Generated), self.f(fr, ni, "W", v)];
                Self::data(self.emit(Op::WhiteNoise { dims: dims(n.enum_prop("noise_dimensions")) }, ins, 2, v), &[F, C])
            }
            "ShaderNodeTexGabor" => {
                let ins = vec![
                    self.texco(fr, ni, v, Implicit::Generated),
                    self.f(fr, ni, "Scale", v),
                    self.f(fr, ni, "Frequency", v),
                    self.f(fr, ni, "Anisotropy", v),
                    self.f(fr, ni, "Orientation 2D", v),
                    self.vec(fr, ni, "Orientation 3D", v),
                ];
                Self::data(self.emit(Op::Gabor { three_d: n.enum_prop("gabor_type") == "3D" }, ins, 3, v), &[F, F, F])
            }
            "ShaderNodeTexIES" => {
                // No IES data: a uniform profile.
                vec![Src::Data(self.f(fr, ni, "Strength", v), F)]
            }
            // ---- Color
            "ShaderNodeBrightContrast" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Bright", v), self.f(fr, ni, "Contrast", v)];
                Self::data(self.emit(Op::BrightContrast, ins, 1, v), &[C])
            }
            "ShaderNodeGamma" => {
                let ins = vec![self.col(fr, ni, "Color", v), self.f(fr, ni, "Gamma", v)];
                Self::data(self.emit(Op::Gamma, ins, 1, v), &[C])
            }
            "ShaderNodeHueSaturation" => {
                let ins = vec![self.f(fr, ni, "Hue", v), self.f(fr, ni, "Saturation", v), self.f(fr, ni, "Value", v), self.f(fr, ni, "Fac", v), self.col(fr, ni, "Color", v)];
                Self::data(self.emit(Op::HueSat, ins, 1, v), &[C])
            }
            "ShaderNodeInvert" => {
                let ins = vec![self.f(fr, ni, "Fac", v), self.col(fr, ni, "Color", v)];
                Self::data(self.emit(Op::Invert, ins, 1, v), &[C])
            }
            "ShaderNodeMix" => {
                let data_type = n.enum_prop("data_type");
                let non_uniform = n.enum_prop("factor_mode") == "NON_UNIFORM";
                let clamp_fac = !matches!(n.prop("clamp_factor"), Some(Prop::Bool(false)));
                let z = self.konst([0.0; 4]);
                match data_type {
                    "FLOAT" => {
                        let ins = vec![self.f(fr, ni, "Factor_Float", v), self.f(fr, ni, "A_Float", v), self.f(fr, ni, "B_Float", v)];
                        let r = self.emit(Op::MixFloat { clamp: clamp_fac }, ins, 1, v);
                        vec![Src::Data(r, F), Src::Data(z, V), Src::Data(z, C)]
                    }
                    "VECTOR" => {
                        let fac = if non_uniform { self.vec(fr, ni, "Factor_Vector", v) } else { self.f(fr, ni, "Factor_Float", v) };
                        let ins = vec![fac, self.vec(fr, ni, "A_Vector", v), self.vec(fr, ni, "B_Vector", v)];
                        let r = self.emit(Op::MixVec { clamp: clamp_fac, non_uniform }, ins, 1, v);
                        vec![Src::Data(z, F), Src::Data(r, V), Src::Data(z, C)]
                    }
                    _ => {
                        let ins = vec![self.f(fr, ni, "Factor_Float", v), self.col(fr, ni, "A_Color", v), self.col(fr, ni, "B_Color", v)];
                        let op = Op::MixColor { blend: Blend::from_name(n.enum_prop("blend_type")), clamp_fac, clamp_result: n.bool_prop("clamp_result") };
                        let r = self.emit(op, ins, 1, v);
                        vec![Src::Data(z, F), Src::Data(z, V), Src::Data(r, C)]
                    }
                }
            }
            "ShaderNodeMixRGB" => {
                let ins = vec![self.f(fr, ni, "Fac", v), self.col(fr, ni, "Color1", v), self.col(fr, ni, "Color2", v)];
                let op = Op::MixRgb { blend: Blend::from_name(n.enum_prop("blend_type")), use_alpha: n.bool_prop("use_alpha"), clamp: n.bool_prop("use_clamp") };
                Self::data(self.emit(op, ins, 1, v), &[C])
            }
            "ShaderNodeRGBCurve" | "ShaderNodeVectorCurve" | "ShaderNodeFloatCurve" => {
                let cm = match n.prop("mapping") {
                    Some(Prop::Curve(c)) => Arc::new(c.clone()),
                    _ => return n.outputs.iter().map(|_| Src::None).collect(),
                };
                match n.idname.as_str() {
                    "ShaderNodeRGBCurve" => {
                        let ins = vec![self.f(fr, ni, "Fac", v), self.col(fr, ni, "Color", v)];
                        Self::data(self.emit(Op::RgbCurve(cm), ins, 1, v), &[C])
                    }
                    "ShaderNodeVectorCurve" => {
                        let ins = vec![self.f(fr, ni, "Fac", v), self.vec(fr, ni, "Vector", v)];
                        Self::data(self.emit(Op::VecCurve(cm), ins, 1, v), &[V])
                    }
                    _ => {
                        let ins = vec![self.f(fr, ni, "Factor", v), self.f(fr, ni, "Value", v)];
                        Self::data(self.emit(Op::FloatCurve(cm), ins, 1, v), &[F])
                    }
                }
            }
            // ---- Vector
            "ShaderNodeBump" => {
                let s = self.f(fr, ni, "Strength", v);
                let d = self.f(fr, ni, "Distance", v);
                let hc = self.f(fr, ni, "Height", v);
                let (vx, vy) = if v == 0 { (1, 2) } else { (v, v) };
                let hx = self.f(fr, ni, "Height", vx);
                let hy = self.f(fr, ni, "Height", vy);
                let nn = self.normal(fr, ni, "Normal", v);
                Self::data(self.emit(Op::Bump { invert: n.bool_prop("invert") }, vec![s, d, hc, hx, hy, nn], 1, v), &[V])
            }
            "ShaderNodeDisplacement" => {
                let ins = vec![self.f(fr, ni, "Height", v), self.f(fr, ni, "Midlevel", v), self.f(fr, ni, "Scale", v), self.normal(fr, ni, "Normal", v)];
                Self::data(self.emit(Op::Displacement { object_space: n.enum_prop("space") != "WORLD" }, ins, 1, v), &[V])
            }
            "ShaderNodeVectorDisplacement" => {
                let ins = vec![self.col(fr, ni, "Vector", v), self.f(fr, ni, "Midlevel", v), self.f(fr, ni, "Scale", v)];
                let space = match n.enum_prop("space") {
                    "OBJECT" => 1,
                    "WORLD" => 2,
                    _ => 0,
                };
                Self::data(self.emit(Op::VecDisplacement { space }, ins, 1, v), &[V])
            }
            "ShaderNodeMapping" => {
                let ty = MappingType::from_name(n.enum_prop("vector_type"));
                let ins = vec![self.vec(fr, ni, "Vector", v), self.vec(fr, ni, "Location", v), self.vec(fr, ni, "Rotation", v), self.vec(fr, ni, "Scale", v)];
                Self::data(self.emit(Op::Mapping(ty), ins, 1, v), &[V])
            }
            "ShaderNodeNormal" => {
                let dir = value_v4(&n.outputs.first().map_or(Value::Vector([0.0, 0.0, 1.0]), |s| s.value.clone()), V);
                let ins = vec![self.konst(dir), self.vec(fr, ni, "Normal", v)];
                Self::data(self.emit(Op::NormalDot, ins, 2, v), &[V, F])
            }
            "ShaderNodeNormalMap" => {
                let space = match n.enum_prop("space") {
                    "OBJECT" => 1,
                    "WORLD" => 2,
                    "BLENDER_OBJECT" => 3,
                    "BLENDER_WORLD" => 4,
                    _ => 0,
                };
                let ins = vec![self.f(fr, ni, "Strength", v), self.col(fr, ni, "Color", v)];
                Self::data(self.emit(Op::NormalMap { space }, ins, 1, v), &[V])
            }
            "ShaderNodeVectorRotate" => {
                let ty = match n.enum_prop("rotation_type") {
                    "X_AXIS" => 1,
                    "Y_AXIS" => 2,
                    "Z_AXIS" => 3,
                    "EULER_XYZ" => 4,
                    _ => 0,
                };
                let ins = vec![self.vec(fr, ni, "Vector", v), self.vec(fr, ni, "Center", v), self.vec(fr, ni, "Axis", v), self.f(fr, ni, "Angle", v), self.vec(fr, ni, "Rotation", v)];
                Self::data(self.emit(Op::VecRotate { ty, invert: n.bool_prop("invert") }, ins, 1, v), &[V])
            }
            "ShaderNodeVectorTransform" => {
                let sp = |s: &str| match s {
                    "OBJECT" => 1,
                    "CAMERA" => 2,
                    _ => 0,
                };
                let ty = match n.enum_prop("vector_type") {
                    "POINT" => 0,
                    "NORMAL" => 2,
                    _ => 1,
                };
                let ins = vec![self.vec(fr, ni, "Vector", v)];
                Self::data(self.emit(Op::VecTransform { ty, from: sp(n.enum_prop("convert_from")), to: sp(n.enum_prop("convert_to")) }, ins, 1, v), &[V])
            }
            // ---- Converter
            "ShaderNodeBlackbody" => {
                let ins = vec![self.f(fr, ni, "Temperature", v)];
                Self::data(self.emit(Op::Blackbody, ins, 1, v), &[C])
            }
            "ShaderNodeWavelength" => {
                let ins = vec![self.f(fr, ni, "Wavelength", v)];
                Self::data(self.emit(Op::Wavelength, ins, 1, v), &[C])
            }
            "ShaderNodeClamp" => {
                let ins = vec![self.f(fr, ni, "Value", v), self.f(fr, ni, "Min", v), self.f(fr, ni, "Max", v)];
                Self::data(self.emit(Op::Clamp { range: n.enum_prop("clamp_type") == "RANGE" }, ins, 1, v), &[F])
            }
            "ShaderNodeValToRGB" => {
                let ramp = match n.prop("color_ramp") {
                    Some(Prop::Ramp(r)) => Arc::new(r.clone()),
                    _ => return vec![Src::None, Src::None],
                };
                let ins = vec![self.f(fr, ni, "Fac", v)];
                Self::data(self.emit(Op::Ramp(ramp), ins, 2, v), &[C, F])
            }
            "ShaderNodeCombineColor" | "ShaderNodeCombineRGB" | "ShaderNodeCombineHSV" => {
                let (ids, mode) = match n.idname.as_str() {
                    "ShaderNodeCombineRGB" => (["R", "G", "B"], 0),
                    "ShaderNodeCombineHSV" => (["H", "S", "V"], 1),
                    _ => (
                        ["Red", "Green", "Blue"],
                        match n.enum_prop("mode") {
                            "HSV" => 1,
                            "HSL" => 2,
                            _ => 0,
                        },
                    ),
                };
                let ins = ids.iter().map(|id| self.f(fr, ni, id, v)).collect();
                Self::data(self.emit(Op::CombineColor(mode), ins, 1, v), &[C])
            }
            "ShaderNodeSeparateColor" | "ShaderNodeSeparateRGB" | "ShaderNodeSeparateHSV" => {
                let (id, mode) = match n.idname.as_str() {
                    "ShaderNodeSeparateRGB" => ("Image", 0),
                    "ShaderNodeSeparateHSV" => ("Color", 1),
                    _ => (
                        "Color",
                        match n.enum_prop("mode") {
                            "HSV" => 1,
                            "HSL" => 2,
                            _ => 0,
                        },
                    ),
                };
                let ins = vec![self.col(fr, ni, id, v)];
                Self::data(self.emit(Op::SeparateColor(mode), ins, 3, v), &[F, F, F])
            }
            "ShaderNodeCombineXYZ" => {
                let ins = vec![self.f(fr, ni, "X", v), self.f(fr, ni, "Y", v), self.f(fr, ni, "Z", v)];
                Self::data(self.emit(Op::CombineXyz, ins, 1, v), &[V])
            }
            "ShaderNodeSeparateXYZ" => {
                let ins = vec![self.vec(fr, ni, "Vector", v)];
                Self::data(self.emit(Op::SeparateXyz, ins, 3, v), &[F, F, F])
            }
            "ShaderNodeMapRange" => {
                let vector = n.enum_prop("data_type") == "FLOAT_VECTOR";
                let ins = if vector {
                    ["Vector", "From_Min_FLOAT3", "From_Max_FLOAT3", "To_Min_FLOAT3", "To_Max_FLOAT3", "Steps_FLOAT3"].iter().map(|id| self.vec(fr, ni, id, v)).collect()
                } else {
                    ["Value", "From Min", "From Max", "To Min", "To Max", "Steps"].iter().map(|id| self.f(fr, ni, id, v)).collect()
                };
                let op = Op::MapRange { interp: MapInterp::from_name(n.enum_prop("interpolation_type")), clamp: !matches!(n.prop("clamp"), Some(Prop::Bool(false))), vector };
                Self::data(self.emit(op, ins, 2, v), &[F, V])
            }
            "ShaderNodeMath" => {
                let op = MathOp::from_name(n.enum_prop("operation"));
                let ins = vec![self.f(fr, ni, "Value", v), self.f(fr, ni, "Value_001", v), self.f(fr, ni, "Value_002", v)];
                Self::data(self.emit(Op::Math { op, clamp: n.bool_prop("use_clamp") }, ins, 1, v), &[F])
            }
            "ShaderNodeVectorMath" => {
                let op = VecOp::from_name(n.enum_prop("operation"));
                let ins = vec![self.vec(fr, ni, "Vector", v), self.vec(fr, ni, "Vector_001", v), self.vec(fr, ni, "Vector_002", v), self.f(fr, ni, "Scale", v)];
                Self::data(self.emit(Op::VecMath(op), ins, 2, v), &[V, F])
            }
            "ShaderNodeRGBToBW" => {
                let ins = vec![self.col(fr, ni, "Color", v)];
                Self::data(self.emit(Op::RgbToBw, ins, 1, v), &[F])
            }
            _ => n.outputs.iter().map(|_| Src::None).collect(),
        }
    }
}

fn active_group_output(g: &NodeTree) -> Option<usize> {
    let mut first = None;
    for (i, n) in g.nodes.iter().enumerate() {
        if n.idname == "NodeGroupOutput" {
            if n.bool_prop("is_active_output") {
                return Some(i);
            }
            first.get_or_insert(i);
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::super::{Closures, Scratch};
    use super::*;
    use crate::math::Vec3;
    use crate::nodes::TreeKind;

    fn tree() -> (NodeTree, usize) {
        let mut t = NodeTree::new("T", TreeKind::Shader);
        let out = t.add("ShaderNodeOutputMaterial", [300.0, 0.0]).unwrap();
        (t, out)
    }

    fn eval(t: &NodeTree, groups: &[NodeTree], ctx: &mut ShadeCtx) -> (Program, Closures) {
        let p = compile_material(t, groups).expect("program");
        let c = p.eval_surface(ctx, &mut Scratch::default());
        (p, c)
    }

    #[test]
    fn constant_math_folds_away() {
        let (mut t, out) = tree();
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        let m = t.add("ShaderNodeMath", [-200.0, 0.0]).unwrap();
        t.nodes[m].set_prop("operation", Prop::Enum("MULTIPLY".into()));
        t.nodes[m].inputs[0].value = Value::Float(3.0);
        t.nodes[m].inputs[1].value = Value::Float(4.0);
        t.link(m, "Value", e, "Strength").unwrap();
        t.link(e, "Emission", out, "Surface").unwrap();
        let (p, c) = eval(&t, &[], &mut ShadeCtx::default());
        assert!(p.surf.is_empty(), "{:?}", p.surf);
        assert!(p.info.emission);
        assert!((c.emission.x - 12.0).abs() < 1e-5, "{:?}", c.emission);
    }

    #[test]
    fn mix_shader_weights_and_muted_nodes() {
        let (mut t, out) = tree();
        let a = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        let b = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        let m = t.add("ShaderNodeMixShader", [0.0, 0.0]).unwrap();
        t.nodes[m].inputs[0].value = Value::Float(0.25);
        t.link(a, "Emission", m, "Shader").unwrap();
        t.link(b, "BSDF", m, "Shader_001").unwrap();
        t.link(m, "Shader", out, "Surface").unwrap();
        let (_, c) = eval(&t, &[], &mut ShadeCtx::default());
        assert!((c.emission.x - 0.75).abs() < 1e-5);
        assert_eq!(c.lobes.len(), 1);
        assert!((c.lobes[0].weight.x - 0.25 * 0.8).abs() < 1e-5);
        // Muting the mix passes its first shader input through.
        t.nodes[m].mute = true;
        let (_, c) = eval(&t, &[], &mut ShadeCtx::default());
        assert!((c.emission.x - 1.0).abs() < 1e-5 && c.lobes.is_empty());
    }

    #[test]
    fn groups_are_inlined() {
        let mut g = NodeTree::new("G", TreeKind::Shader);
        g.inputs.push(crate::nodes::InterfaceSocket { name: "In".into(), ty: SocketType::Float, default: Value::Float(1.0), min: f32::MIN, max: f32::MAX });
        g.outputs.push(crate::nodes::InterfaceSocket { name: "Out".into(), ty: SocketType::Float, default: Value::Float(0.0), min: f32::MIN, max: f32::MAX });
        let gi = g.add("NodeGroupInput", [0.0, 0.0]).unwrap();
        let go = g.add("NodeGroupOutput", [0.0, 0.0]).unwrap();
        let (gi_n, go_n) = (g.nodes[gi].clone(), g.nodes[go].clone());
        let mut gi_n = gi_n;
        let mut go_n = go_n;
        crate::nodes::sync_group_sockets(&mut gi_n, &g);
        crate::nodes::sync_group_sockets(&mut go_n, &g);
        g.nodes[gi] = gi_n;
        g.nodes[go] = go_n;
        let m = g.add("ShaderNodeMath", [0.0, 0.0]).unwrap();
        g.nodes[m].set_prop("operation", Prop::Enum("ADD".into()));
        g.nodes[m].inputs[1].value = Value::Float(2.0);
        g.link_sockets(gi, 0, m, 0).unwrap();
        g.link_sockets(m, 0, go, 0).unwrap();

        let (mut t, out) = tree();
        let gn = t.add("ShaderNodeGroup", [0.0, 0.0]).unwrap();
        t.nodes[gn].set_prop("node_tree", Prop::Id(Some(0)));
        let mut node = t.nodes[gn].clone();
        crate::nodes::sync_group_sockets(&mut node, &g);
        t.nodes[gn] = node;
        t.nodes[gn].inputs[0].value = Value::Float(5.0);
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.link_sockets(gn, 0, e, 1).unwrap();
        t.link(e, "Emission", out, "Surface").unwrap();
        let (_, c) = eval(&t, &[g], &mut ShadeCtx::default());
        assert!((c.emission.x - 7.0).abs() < 1e-5, "{:?}", c.emission);
    }

    #[test]
    fn checker_reads_generated_coordinates() {
        let (mut t, out) = tree();
        let d = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        let c = t.add("ShaderNodeTexChecker", [0.0, 0.0]).unwrap();
        t.nodes[c].set_input("Scale", Value::Float(2.0));
        t.link(c, "Color", d, "Color").unwrap();
        t.link(d, "BSDF", out, "Surface").unwrap();
        let mut ctx = ShadeCtx { p: Vec3::new(0.1, 0.1, 0.1), ..Default::default() };
        let (_, a) = eval(&t, &[], &mut ctx);
        let mut ctx = ShadeCtx { p: Vec3::new(0.6, 0.1, 0.1), ..Default::default() };
        let (_, b) = eval(&t, &[], &mut ctx);
        assert!((a.lobes[0].weight.x - b.lobes[0].weight.x).abs() > 0.3);
    }

    #[test]
    fn bump_tilts_the_normal() {
        let (mut t, out) = tree();
        let d = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        let bump = t.add("ShaderNodeBump", [0.0, 0.0]).unwrap();
        let geo = t.add("ShaderNodeSeparateXYZ", [0.0, 0.0]).unwrap();
        let pos = t.add("ShaderNodeNewGeometry", [0.0, 0.0]).unwrap();
        t.link(pos, "Position", geo, "Vector").unwrap();
        t.link(geo, "X", bump, "Height").unwrap();
        t.link(bump, "Normal", d, "Normal").unwrap();
        t.link(d, "BSDF", out, "Surface").unwrap();
        let mut ctx = ShadeCtx { dpdx: Vec3::new(1e-3, 0.0, 0.0), dpdy: Vec3::new(0.0, 1e-3, 0.0), ..Default::default() };
        let (_, c) = eval(&t, &[], &mut ctx);
        let n = c.lobes[0].n;
        // Height rises along +X, so the normal leans to -X (45 degrees).
        assert!((n.x + core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3, "{n:?}");
    }
}
