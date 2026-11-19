// SPDX-License-Identifier: GPL-3.0-only
//! Lazy fields, as Blender's `fn::Field`: a field is a function evaluated
//! per element of a geometry component's domain. Inputs (Position, Index,
//! named attributes, ...) read the context; function nodes combine fields
//! element by element; context-dependent operations (Evaluate on Domain,
//! Evaluate at Index, Accumulate, Sample ...) evaluate their own inputs on
//! other domains or other geometry.
//!
//! An input returns its values on its native domain (positions on points,
//! face areas on faces); [`eval`] moves them to the requested domain with
//! Blender's attribute domain interpolation ([`super::topo::adapt_mesh`]).
use super::attributes::{AttrData, AttrType, Domain, Val};
use super::curves::Curves;
use super::func::NodeFn;
use super::topo::{adapt_mesh, read_mesh_attr, MeshTopo};
use super::{Instances, PointCloud};
use crate::math::Vec3;
use crate::mesh::Mesh;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

pub type Field = Arc<FieldNode>;
/// Context input: values on some domain, or None for "not available here"
/// (the field's default is used).
pub type InputFn = Arc<dyn Fn(&Ctx, Domain) -> Option<(Domain, Vec<Val>)> + Send + Sync>;
/// A field that evaluates other fields itself.
pub type SpecialFn = Arc<dyn Fn(&Ctx, Domain, &mut Cache) -> Vec<Val> + Send + Sync>;

pub struct FnOp {
    pub f: NodeFn,
    pub inputs: Vec<Field>,
}

pub enum FieldNode {
    Const(Val),
    Input { name: String, f: InputFn, default: Val },
    Fn { op: Arc<FnOp>, output: usize, default: Val },
    Special { name: String, f: SpecialFn, default: Val },
}

impl std::fmt::Debug for FieldNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldNode::Const(v) => write!(f, "Const({v:?})"),
            FieldNode::Input { name, .. } => write!(f, "Input({name})"),
            FieldNode::Fn { output, .. } => write!(f, "Fn(out {output})"),
            FieldNode::Special { name, .. } => write!(f, "Special({name})"),
        }
    }
}

impl FieldNode {
    /// The value used where the field has no context (and its type).
    pub fn default_val(&self) -> Val {
        match self {
            FieldNode::Const(v) => *v,
            FieldNode::Input { default, .. } | FieldNode::Fn { default, .. } | FieldNode::Special { default, .. } => *default,
        }
    }

    pub fn is_const(&self) -> bool {
        matches!(self, FieldNode::Const(_))
    }
}

pub fn konst(v: Val) -> Field {
    Arc::new(FieldNode::Const(v))
}

pub fn input(name: &str, default: Val, f: impl Fn(&Ctx, Domain) -> Option<(Domain, Vec<Val>)> + Send + Sync + 'static) -> Field {
    Arc::new(FieldNode::Input { name: name.into(), f: Arc::new(f), default })
}

pub fn special(name: &str, default: Val, f: impl Fn(&Ctx, Domain, &mut Cache) -> Vec<Val> + Send + Sync + 'static) -> Field {
    Arc::new(FieldNode::Special { name: name.into(), f: Arc::new(f), default })
}

/// A one-output function of `inputs`.
pub fn map(inputs: Vec<Field>, default: Val, f: impl Fn(&[Val]) -> Val + Send + Sync + 'static) -> Field {
    let op = Arc::new(FnOp { f: Arc::new(move |v: &[Val]| vec![f(v)]), inputs });
    Arc::new(FieldNode::Fn { op, output: 0, default })
}

/// The component a field is evaluated on.
#[derive(Clone, Copy)]
pub enum Ctx<'a> {
    /// No geometry: a single value.
    None,
    Mesh(&'a Mesh, &'a MeshTopo),
    Curves(&'a Curves),
    Points(&'a PointCloud),
    Instances(&'a Instances),
}

impl<'a> Ctx<'a> {
    pub fn size(&self, d: Domain) -> usize {
        match self {
            Ctx::None => 1,
            Ctx::Mesh(m, t) => t.size(m, d),
            Ctx::Curves(c) => c.size(d),
            Ctx::Points(p) => {
                if d == Domain::Point {
                    p.positions.len()
                } else {
                    0
                }
            }
            Ctx::Instances(i) => {
                if d == Domain::Instance {
                    i.len()
                } else {
                    0
                }
            }
        }
    }

    pub fn adapt(&self, vals: &[Val], from: Domain, to: Domain) -> Vec<Val> {
        if from == to {
            return vals.to_vec();
        }
        match self {
            Ctx::Mesh(m, t) => adapt_mesh(m, t, vals, from, to),
            Ctx::Curves(c) => c.adapt(vals, from, to),
            _ => {
                let def = vals.first().map(|v| AttrType::of_val(v).default_val()).unwrap_or(Val::Float(0.0));
                vec![def; self.size(to)]
            }
        }
    }

    /// A named attribute (built-ins included).
    pub fn read_attr(&self, name: &str) -> Option<(Domain, AttrData)> {
        match self {
            Ctx::None => None,
            Ctx::Mesh(m, t) => read_mesh_attr(m, t, name),
            Ctx::Curves(c) => {
                if name == "position" {
                    return Some((Domain::Point, AttrData::Float3(c.positions.clone())));
                }
                c.attributes.get(name).filter(|a| a.data.len() == c.size(a.domain)).map(|a| (a.domain, a.data.clone()))
            }
            Ctx::Points(p) => {
                if name == "position" {
                    return Some((Domain::Point, AttrData::Float3(p.positions.clone())));
                }
                p.attributes.get(name).filter(|a| a.data.len() == p.positions.len()).map(|a| (a.domain, a.data.clone()))
            }
            Ctx::Instances(i) => {
                if name == "position" {
                    return Some((Domain::Instance, AttrData::Float3(i.transforms.iter().map(|t| t.translation_part()).collect())));
                }
                i.attributes.get(name).filter(|a| a.data.len() == i.len()).map(|a| (Domain::Instance, a.data.clone()))
            }
        }
    }

    /// The domains this component has.
    pub fn domains(&self) -> &'static [Domain] {
        match self {
            Ctx::None => &[Domain::Point],
            Ctx::Mesh(..) => &[Domain::Point, Domain::Edge, Domain::Face, Domain::Corner],
            Ctx::Curves(_) => &[Domain::Point, Domain::Curve],
            Ctx::Points(_) => &[Domain::Point],
            Ctx::Instances(_) => &[Domain::Instance],
        }
    }
}

/// Results already computed while evaluating fields on one component.
#[derive(Default)]
pub struct Cache {
    vals: HashMap<(usize, Domain), Rc<Vec<Val>>>,
    fns: HashMap<(usize, Domain), Rc<Vec<Vec<Val>>>>,
    // Keys are addresses: hold every keyed field so none is freed and its
    // address reused by another field while the cache lives.
    keep: Vec<Field>,
    keep_ops: Vec<Arc<FnOp>>,
}

/// Evaluate `f` for every element of domain `d`.
pub fn eval(f: &Field, ctx: &Ctx, d: Domain, cache: &mut Cache) -> Rc<Vec<Val>> {
    let n = ctx.size(d);
    let key = (Arc::as_ptr(f) as *const u8 as usize, d);
    if let Some(v) = cache.vals.get(&key) {
        return v.clone();
    }
    let out: Vec<Val> = match &**f {
        FieldNode::Const(v) => vec![*v; n],
        FieldNode::Input { f, default, .. } => match f(ctx, d) {
            Some((src, vals)) => {
                let vals = if src == d { vals } else { ctx.adapt(&vals, src, d) };
                if vals.len() == n {
                    vals.into_iter().map(|v| v.like(default)).collect()
                } else {
                    vec![*default; n]
                }
            }
            None => vec![*default; n],
        },
        FieldNode::Fn { op, output, default } => {
            let k = (Arc::as_ptr(op) as *const u8 as usize, d);
            let all = match cache.fns.get(&k) {
                Some(a) => a.clone(),
                None => {
                    let ins: Vec<Rc<Vec<Val>>> = op.inputs.iter().map(|x| eval(x, ctx, d, cache)).collect();
                    let mut row = vec![Val::Float(0.0); ins.len()];
                    let res: Vec<Vec<Val>> = (0..n)
                        .map(|i| {
                            for (j, c) in ins.iter().enumerate() {
                                row[j] = c.get(i).copied().unwrap_or(Val::Float(0.0));
                            }
                            (op.f)(&row)
                        })
                        .collect();
                    let r = Rc::new(res);
                    cache.fns.insert(k, r.clone());
                    cache.keep_ops.push(op.clone());
                    r
                }
            };
            all.iter().map(|o| o.get(*output).copied().unwrap_or(*default)).collect()
        }
        FieldNode::Special { f, default, .. } => {
            let mut v = f(ctx, d, cache);
            v.resize(n, *default);
            v
        }
    };
    let r = Rc::new(out);
    cache.vals.insert(key, r.clone());
    cache.keep.push(f.clone());
    r
}

/// The field's value without geometry (Blender's field-to-single-value).
pub fn eval_single(f: &Field) -> Val {
    if let FieldNode::Const(v) = &**f {
        return *v;
    }
    let v = eval(f, &Ctx::None, Domain::Point, &mut Cache::default());
    v.first().copied().unwrap_or(f.default_val())
}

/// Indices of the elements where a selection field is true.
pub fn selected(sel: &Field, ctx: &Ctx, d: Domain, cache: &mut Cache) -> Vec<bool> {
    eval(sel, ctx, d, cache).iter().map(|v| v.b()).collect()
}

// ---------------------------------------------------------------------------
// Built-in inputs

pub fn position() -> Field {
    input("Position", Val::Vector(Vec3::ZERO), |ctx, _| match ctx {
        Ctx::Mesh(m, _) => Some((Domain::Point, m.positions.iter().map(|&p| Val::Vector(p)).collect())),
        Ctx::Curves(c) => Some((Domain::Point, c.positions.iter().map(|&p| Val::Vector(p)).collect())),
        Ctx::Points(p) => Some((Domain::Point, p.positions.iter().map(|&p| Val::Vector(p)).collect())),
        Ctx::Instances(i) => Some((Domain::Instance, i.transforms.iter().map(|t| Val::Vector(t.translation_part())).collect())),
        Ctx::None => None,
    })
}

/// Blender's vertex normals: face normals summed with corner-angle weights.
pub fn vertex_normals(m: &Mesh) -> Vec<Vec3> {
    let mut acc = vec![Vec3::ZERO; m.positions.len()];
    for (fi, f) in m.faces.iter().enumerate() {
        let n = m.face_normal(fi);
        let k = f.verts.len();
        for i in 0..k {
            let (a, b, c) = (f.verts[(i + k - 1) % k], f.verts[i], f.verts[(i + 1) % k]);
            let (p0, p1, p2) = (m.positions[a as usize], m.positions[b as usize], m.positions[c as usize]);
            let (e0, e1) = ((p0 - p1).normalize(), (p2 - p1).normalize());
            let ang = e0.dot(e1).clamp(-1.0, 1.0).acos();
            if let Some(x) = acc.get_mut(b as usize) {
                *x += n * ang;
            }
        }
    }
    acc.into_iter().zip(&m.positions).map(|(n, p)| if n.length_squared() > 0.0 { n.normalize() } else { p.normalize() }).collect()
}

pub fn normal() -> Field {
    input("Normal", Val::Vector(Vec3::Z), |ctx, d| match ctx {
        Ctx::Mesh(m, t) => Some(match d {
            Domain::Face => (Domain::Face, (0..m.faces.len()).map(|f| Val::Vector(m.face_normal(f))).collect()),
            Domain::Corner => {
                let vn = vertex_normals(m);
                (Domain::Corner, (0..t.corner_count()).map(|c| {
                    let f = t.corner_face[c] as usize;
                    Val::Vector(if m.faces[f].smooth { vn[t.corner_vert[c] as usize] } else { m.face_normal(f) })
                }).collect())
            }
            Domain::Edge => {
                let vn = vertex_normals(m);
                (Domain::Edge, t.edges.iter().enumerate().map(|(i, e)| {
                    let fs = &t.edge_faces[i];
                    let n = if fs.is_empty() { vn[e[0] as usize] + vn[e[1] as usize] } else { fs.iter().fold(Vec3::ZERO, |a, &f| a + m.face_normal(f as usize)) };
                    Val::Vector(n.normalize())
                }).collect())
            }
            _ => (Domain::Point, vertex_normals(m).into_iter().map(Val::Vector).collect()),
        }),
        Ctx::Curves(c) => {
            let mut out = vec![Val::Vector(Vec3::Z); c.points_num()];
            for ci in 0..c.curves_num() {
                let ev = c.evaluate(ci);
                for (k, &(a, _, f)) in ev.src.iter().enumerate() {
                    if f == 0.0 && a < out.len() {
                        out[a] = Val::Vector(ev.normals[k]);
                    }
                }
            }
            Some((Domain::Point, out))
        }
        _ => None,
    })
}

pub fn index() -> Field {
    input("Index", Val::Int(0), |ctx, d| Some((d, (0..ctx.size(d) as i32).map(Val::Int).collect())))
}

/// Blender's ID: the `id` attribute, else the index.
pub fn id() -> Field {
    input("ID", Val::Int(0), |ctx, d| match ctx.read_attr("id") {
        Some((dom, data)) => Some((dom, data.to_vals())),
        None => Some((d, (0..ctx.size(d) as i32).map(Val::Int).collect())),
    })
}

pub fn named_attribute(name: &str, ty: AttrType) -> Field {
    let n = name.to_string();
    input(&format!("Attribute {name}"), ty.default_val(), move |ctx, _| ctx.read_attr(&n).map(|(d, data)| (d, data.to_vals().into_iter().map(|v| ty.convert(v)).collect())))
}

pub fn radius() -> Field {
    input("Radius", Val::Float(0.0), |ctx, _| match ctx {
        Ctx::Curves(c) => Some((Domain::Point, (0..c.points_num()).map(|i| Val::Float(c.radius(i))).collect())),
        Ctx::Points(p) => Some((Domain::Point, p.radius().into_iter().map(Val::Float).collect())),
        _ => None,
    })
}

/// Whether a named attribute exists on the context (for Named Attribute's
/// "Exists" output).
pub fn attribute_exists(name: &str) -> Field {
    let n = name.to_string();
    input("Exists", Val::Bool(false), move |ctx, d| Some((d, vec![Val::Bool(ctx.read_attr(&n).is_some()); ctx.size(d)])))
}
