// SPDX-License-Identifier: GPL-3.0-only
//! Geometry nodes: Blender's `GeometrySet` and the node tree evaluator.
//!
//! A [`Geometry`] holds up to one component of each kind: a [`Mesh`] (with
//! generic attributes on its point, edge, face and face-corner domains),
//! [`Curves`], a [`PointCloud`] and [`Instances`] (references to geometry,
//! objects or collections with transforms). Fields ([`field`]) are lazy
//! per-element functions evaluated on a component's domain; the node tree
//! evaluator ([`eval`]) runs a `GeometryNodeTree` with Blender's semantics,
//! node groups, repeat zones and simulation zones. The Nodes modifier
//! ([`crate::modifiers::Modifier::Nodes`]) calls [`eval::run_modifier`].
use crate::math::{Aabb, Mat4, Vec3};
use crate::mesh::Mesh;

pub mod attributes;
pub mod curves;
pub mod eval;
pub mod field;
pub mod func;
mod meshops;
mod nodes_curve;
mod nodes_geo;
mod nodes_mesh;
pub mod primitives;
mod sample;
pub mod topo;
#[cfg(test)]
mod tests;

pub use attributes::{AttrData, AttrType, Attribute, Attributes, Domain, Val};
pub use curves::Curves;
pub use eval::{clear_simulation_cache, evaluate_group, run_modifier, EvalOptions, SocketValue};
pub use topo::MeshTopo;

/// A new geometry node group as Blender's "New" button makes it: a Geometry
/// input and output, the Group Input linked straight to the Group Output.
pub fn new_group(name: &str) -> crate::nodes::NodeTree {
    use crate::nodes::{edit, InterfaceSocket, NodeTree, SocketType, TreeKind, Value};
    let mut t = NodeTree::new(name, TreeKind::Geometry);
    let iface = || InterfaceSocket { name: "Geometry".into(), ty: SocketType::Geometry, default: Value::None, min: f32::MIN, max: f32::MAX };
    t.inputs.push(iface());
    t.outputs.push(iface());
    let mut gi = edit::group_input_node(&t);
    gi.location = [-200.0, 0.0];
    let mut go = edit::group_output_node(&t);
    go.location = [200.0, 0.0];
    t.nodes.push(gi);
    t.nodes.push(go);
    let _ = t.link_sockets(0, 0, 1, 0);
    t
}

/// Points without faces (Blender's `PointCloud`); `radius` is a built-in
/// float attribute.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointCloud {
    pub positions: Vec<Vec3>,
    pub attributes: Attributes,
}

impl PointCloud {
    pub fn radius(&self) -> Vec<f32> {
        match self.attributes.get("radius") {
            Some(a) if a.data.len() == self.positions.len() => a.data.to_vals().iter().map(|v| v.f()).collect(),
            _ => vec![0.01; self.positions.len()],
        }
    }
}

/// What an instance shows.
#[derive(Clone, Debug, PartialEq)]
pub enum InstanceRef {
    Geometry(Box<Geometry>),
    /// A scene object (its evaluated geometry).
    Object(usize),
    /// A scene collection (its objects, relative to the collection).
    Collection(usize),
}

/// Instances (Blender's `bke::Instances`): a list of references and, per
/// instance, the reference it shows (`handles`), a transform and attributes
/// on the instance domain.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Instances {
    pub references: Vec<InstanceRef>,
    pub handles: Vec<u32>,
    pub transforms: Vec<Mat4>,
    pub attributes: Attributes,
}

impl Instances {
    pub fn len(&self) -> usize {
        self.transforms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty()
    }

    /// The reference's handle, reusing an equal one.
    pub fn add_reference(&mut self, r: InstanceRef) -> u32 {
        if let Some(i) = self.references.iter().position(|x| *x == r) {
            return i as u32;
        }
        self.references.push(r);
        (self.references.len() - 1) as u32
    }

    pub fn add(&mut self, handle: u32, transform: Mat4) {
        self.handles.push(handle);
        self.transforms.push(transform);
        let n = self.transforms.len();
        for a in &mut self.attributes.list {
            a.data.resize(n);
        }
    }

    /// Keep instances `keep` (in order).
    pub fn gather(&self, keep: &[usize]) -> Instances {
        let mut out = Instances { references: self.references.clone(), ..Default::default() };
        out.handles = keep.iter().map(|&i| self.handles[i]).collect();
        out.transforms = keep.iter().map(|&i| self.transforms[i]).collect();
        out.attributes = self.attributes.clone();
        out.attributes.gather_domain(Domain::Instance, keep);
        out
    }

    pub fn join(&mut self, o: &Instances) {
        let (n, m) = (self.len(), o.len());
        self.attributes.join(&o.attributes, |_| (n, m));
        let map: Vec<u32> = o.references.iter().map(|r| self.add_reference(r.clone())).collect();
        self.handles.extend(o.handles.iter().map(|&h| map.get(h as usize).copied().unwrap_or(0)));
        self.transforms.extend_from_slice(&o.transforms);
    }
}

/// A geometry set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geometry {
    pub mesh: Option<Mesh>,
    pub curves: Option<Curves>,
    pub points: Option<PointCloud>,
    pub instances: Option<Instances>,
    /// Volume grids are not supported; the flag keeps a Volume component
    /// passing through so Separate Components reports it.
    pub volume: bool,
}

/// The component kinds of a geometry set (Blender's `GeometryComponent::Type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Component {
    Mesh,
    Curves,
    Points,
    Instances,
}

impl Component {
    pub const ALL: [Component; 4] = [Component::Mesh, Component::Points, Component::Curves, Component::Instances];

    pub fn domains(self) -> &'static [Domain] {
        match self {
            Component::Mesh => &[Domain::Point, Domain::Edge, Domain::Face, Domain::Corner],
            Component::Curves => &[Domain::Point, Domain::Curve],
            Component::Points => &[Domain::Point],
            Component::Instances => &[Domain::Instance],
        }
    }

    pub fn from_name(s: &str) -> Option<Component> {
        Some(match s {
            "MESH" => Component::Mesh,
            "CURVE" | "CURVES" => Component::Curves,
            "POINTCLOUD" | "POINTS" => Component::Points,
            "INSTANCES" => Component::Instances,
            _ => return None,
        })
    }
}

impl Geometry {
    pub fn from_mesh(m: Mesh) -> Geometry {
        Geometry { mesh: Some(m), ..Default::default() }
    }

    pub fn from_curves(c: Curves) -> Geometry {
        Geometry { curves: Some(c), ..Default::default() }
    }

    pub fn from_points(p: PointCloud) -> Geometry {
        Geometry { points: Some(p), ..Default::default() }
    }

    pub fn from_instances(i: Instances) -> Geometry {
        Geometry { instances: Some(i), ..Default::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.mesh.as_ref().is_none_or(|m| m.positions.is_empty())
            && self.curves.as_ref().is_none_or(|c| c.points_num() == 0)
            && self.points.as_ref().is_none_or(|p| p.positions.is_empty())
            && self.instances.as_ref().is_none_or(|i| i.is_empty())
    }

    pub fn has(&self, c: Component) -> bool {
        match c {
            Component::Mesh => self.mesh.is_some(),
            Component::Curves => self.curves.is_some(),
            Component::Points => self.points.is_some(),
            Component::Instances => self.instances.is_some(),
        }
    }

    /// Only component `c`.
    pub fn only(&self, c: Component) -> Geometry {
        match c {
            Component::Mesh => Geometry { mesh: self.mesh.clone(), ..Default::default() },
            Component::Curves => Geometry { curves: self.curves.clone(), ..Default::default() },
            Component::Points => Geometry { points: self.points.clone(), ..Default::default() },
            Component::Instances => Geometry { instances: self.instances.clone(), ..Default::default() },
        }
    }

    pub fn remove(&mut self, c: Component) {
        match c {
            Component::Mesh => self.mesh = None,
            Component::Curves => self.curves = None,
            Component::Points => self.points = None,
            Component::Instances => self.instances = None,
        }
    }

    /// Element count of a component's domain.
    pub fn domain_size(&self, c: Component, d: Domain) -> usize {
        match c {
            Component::Mesh => self.mesh.as_ref().map_or(0, |m| match d {
                Domain::Point => m.positions.len(),
                Domain::Face => m.faces.len(),
                Domain::Corner => m.faces.iter().map(|f| f.verts.len()).sum(),
                Domain::Edge => m.edges().len(),
                _ => 0,
            }),
            Component::Curves => self.curves.as_ref().map_or(0, |x| x.size(d)),
            Component::Points => self.points.as_ref().map_or(0, |p| if d == Domain::Point { p.positions.len() } else { 0 }),
            Component::Instances => self.instances.as_ref().map_or(0, |i| if d == Domain::Instance { i.len() } else { 0 }),
        }
    }

    /// Transform every component by `m` (instances get `m` on the left).
    pub fn transform(&mut self, m: &Mat4) {
        if let Some(x) = &mut self.mesh {
            x.transform(m);
        }
        if let Some(c) = &mut self.curves {
            for p in &mut c.positions {
                *p = m.transform_point(*p);
            }
            for name in ["handle_left", "handle_right"] {
                if let Some(a) = c.attributes.get_mut(name) {
                    for i in 0..a.data.len() {
                        let v = a.data.get(i).v();
                        a.data.set(i, Val::Vector(m.transform_point(v)));
                    }
                }
            }
            let s = m.mat3().determinant().abs().cbrt();
            if (s - 1.0).abs() > 1e-6 {
                if let Some(a) = c.attributes.get_mut("radius") {
                    for i in 0..a.data.len() {
                        let v = a.data.get(i).f();
                        a.data.set(i, Val::Float(v * s));
                    }
                }
            }
        }
        if let Some(p) = &mut self.points {
            for x in &mut p.positions {
                *x = m.transform_point(*x);
            }
        }
        if let Some(i) = &mut self.instances {
            for t in &mut i.transforms {
                *t = *m * *t;
            }
        }
    }

    /// Bounds of the realized geometry (instances included).
    pub fn bounds(&self, resolve: &dyn Fn(&InstanceRef) -> Option<Geometry>) -> Aabb {
        let mut b = Aabb::EMPTY;
        let g = realize(self, resolve);
        for p in g.mesh.iter().flat_map(|m| m.positions.iter()).chain(g.curves.iter().flat_map(|c| c.positions.iter())).chain(g.points.iter().flat_map(|p| p.positions.iter())) {
            b.grow(*p);
        }
        b
    }

    /// Join another geometry set into this one, component by component.
    pub fn join(&mut self, o: Geometry) {
        if let Some(m) = o.mesh {
            match &mut self.mesh {
                Some(a) => *a = join_meshes(a, &m),
                None => self.mesh = Some(m),
            }
        }
        if let Some(c) = o.curves {
            match &mut self.curves {
                Some(a) => a.join(&c),
                None => self.curves = Some(c),
            }
        }
        if let Some(p) = o.points {
            match &mut self.points {
                Some(a) => {
                    let (n, k) = (a.positions.len(), p.positions.len());
                    a.attributes.join(&p.attributes, |_| (n, k));
                    a.positions.extend_from_slice(&p.positions);
                }
                None => self.points = Some(p),
            }
        }
        if let Some(i) = o.instances {
            match &mut self.instances {
                Some(a) => a.join(&i),
                None => self.instances = Some(i),
            }
        }
        self.volume |= o.volume;
    }
}

/// Join two meshes with their attributes (edge attributes follow their
/// vertex pairs).
pub fn join_meshes(a: &Mesh, b: &Mesh) -> Mesh {
    let mut out = a.clone();
    out.attributes = Attributes::default();
    out.append(b);
    let ta = MeshTopo::new(a);
    let tb = MeshTopo::new(b);
    let to = MeshTopo::new(&out);
    let off = a.positions.len() as u32;
    let mut attrs = a.attributes.clone();
    let sizes = |d: Domain| (ta.size(a, d), tb.size(b, d));
    // Join everything but edges by concatenation; redo edges by key.
    let mut no_edges_a = attrs.clone();
    no_edges_a.list.retain(|x| x.domain != Domain::Edge);
    let mut no_edges_b = b.attributes.clone();
    no_edges_b.list.retain(|x| x.domain != Domain::Edge);
    no_edges_a.join(&no_edges_b, sizes);
    let mut names: Vec<(String, AttrType)> = a.attributes.list.iter().chain(b.attributes.list.iter()).filter(|x| x.domain == Domain::Edge).map(|x| (x.name.clone(), x.data.ty())).collect();
    names.dedup_by(|x, y| x.0 == y.0);
    for (name, ty) in names {
        let mut d = AttrData::new(ty, to.edges.len());
        for (i, e) in to.edges.iter().enumerate() {
            let v = if e[0] < off {
                ta.edge_index(e[0], e[1]).and_then(|k| a.attributes.get(&name).filter(|x| x.domain == Domain::Edge && (k as usize) < x.data.len()).map(|x| x.data.get(k as usize)))
            } else {
                tb.edge_index(e[0] - off, e[1] - off).and_then(|k| b.attributes.get(&name).filter(|x| x.domain == Domain::Edge && (k as usize) < x.data.len()).map(|x| x.data.get(k as usize)))
            };
            if let Some(v) = v {
                d.set(i, v);
            }
        }
        no_edges_a.set(&name, Domain::Edge, d);
    }
    attrs = no_edges_a;
    out.attributes = attrs;
    out
}

/// Realize instances (recursively) into real geometry, Blender's Realize
/// Instances: instance-domain attributes move to the points (meshes, point
/// clouds) and curves of each realized instance.
pub fn realize(g: &Geometry, resolve: &dyn Fn(&InstanceRef) -> Option<Geometry>) -> Geometry {
    realize_depth(g, resolve, 0)
}

fn realize_depth(g: &Geometry, resolve: &dyn Fn(&InstanceRef) -> Option<Geometry>, depth: u32) -> Geometry {
    let mut out = Geometry { mesh: g.mesh.clone(), curves: g.curves.clone(), points: g.points.clone(), instances: None, volume: g.volume };
    let Some(inst) = &g.instances else { return out };
    if depth > 16 {
        return out;
    }
    let resolved: Vec<Option<Geometry>> = inst
        .references
        .iter()
        .map(|r| match r {
            InstanceRef::Geometry(x) => Some(realize_depth(x, resolve, depth + 1)),
            other => resolve(other).map(|x| realize_depth(&x, resolve, depth + 1)),
        })
        .collect();
    for i in 0..inst.len() {
        let Some(Some(src)) = resolved.get(inst.handles[i] as usize) else { continue };
        let mut part = src.clone();
        part.transform(&inst.transforms[i]);
        // Instance attributes onto the realized elements.
        for a in &inst.attributes.list {
            if a.data.len() != inst.len() {
                continue;
            }
            let v = a.data.get(i);
            let ty = a.data.ty();
            if let Some(m) = &mut part.mesh {
                if !m.attributes.contains(&a.name) {
                    m.attributes.set(&a.name, Domain::Point, AttrData::from_vals(ty, &vec![v; m.positions.len()]));
                }
            }
            if let Some(p) = &mut part.points {
                if !p.attributes.contains(&a.name) {
                    p.attributes.set(&a.name, Domain::Point, AttrData::from_vals(ty, &vec![v; p.positions.len()]));
                }
            }
            if let Some(c) = &mut part.curves {
                if !c.attributes.contains(&a.name) {
                    let n = c.curves_num();
                    c.attributes.set(&a.name, Domain::Curve, AttrData::from_vals(ty, &vec![v; n]));
                }
            }
        }
        out.join(part);
    }
    out
}

/// How a mesh maps to the mesh it was built from: for each new element, the
/// weighted old elements it takes its attributes from.
#[derive(Clone, Debug, Default)]
pub struct MeshMaps {
    pub verts: Vec<Vec<(usize, f32)>>,
    pub faces: Vec<Vec<(usize, f32)>>,
    pub corners: Vec<Vec<(usize, f32)>>,
    /// Explicit edge sources; when `None`, edges whose two vertices come
    /// from single old vertices joined by an old edge take that edge.
    pub edges: Option<Vec<Vec<(usize, f32)>>>,
}

impl MeshMaps {
    pub fn single(v: &[usize]) -> Vec<Vec<(usize, f32)>> {
        v.iter().map(|&i| vec![(i, 1.0)]).collect()
    }
}

/// Carry `old`'s generic attributes, vertex groups and edge flags over to
/// `new` along `maps`.
pub fn transfer_attributes(old: &Mesh, new: &mut Mesh, maps: &MeshMaps) {
    let ot = MeshTopo::new(old);
    let nt = MeshTopo::new(new);
    let mut attrs = old.attributes.clone();
    attrs.retain_valid(|d| ot.size(old, d));
    attrs.interp_domain(Domain::Point, &maps.verts);
    attrs.interp_domain(Domain::Face, &maps.faces);
    attrs.interp_domain(Domain::Corner, &maps.corners);
    let edge_src: Vec<Vec<(usize, f32)>> = match &maps.edges {
        Some(e) => e.clone(),
        None => nt
            .edges
            .iter()
            .map(|e| {
                let (a, b) = (maps.verts.get(e[0] as usize), maps.verts.get(e[1] as usize));
                match (a, b) {
                    (Some(a), Some(b)) if a.len() == 1 && b.len() == 1 => ot.edge_index(a[0].0 as u32, b[0].0 as u32).map(|k| vec![(k as usize, 1.0)]).unwrap_or_default(),
                    _ => Vec::new(),
                }
            })
            .collect(),
    };
    attrs.interp_domain(Domain::Edge, &edge_src);
    for (d, n) in [(Domain::Point, nt.size(new, Domain::Point)), (Domain::Edge, nt.edges.len()), (Domain::Face, new.faces.len()), (Domain::Corner, nt.corner_count())] {
        for a in attrs.list.iter_mut().filter(|a| a.domain == d) {
            a.data.resize(n);
        }
    }
    new.attributes = attrs;
    // Vertex groups.
    if !old.vertex_groups.is_empty() {
        new.vertex_groups = old.vertex_groups.clone();
        new.weights = maps
            .verts
            .iter()
            .map(|src| {
                let mut w: Vec<(u32, f32)> = Vec::new();
                for &(v, f) in src {
                    for &(g, x) in old.vertex_weights(v) {
                        match w.iter_mut().find(|e| e.0 == g) {
                            Some(e) => e.1 += x * f,
                            None => w.push((g, x * f)),
                        }
                    }
                }
                w
            })
            .collect();
    }
    // Sharp edges and seams.
    let flags = |set: &std::collections::BTreeSet<(u32, u32)>| -> std::collections::BTreeSet<(u32, u32)> {
        if set.is_empty() {
            return Default::default();
        }
        nt.edges
            .iter()
            .zip(&edge_src)
            .filter(|(_, s)| s.len() == 1 && ot.edges.get(s[0].0).is_some_and(|e| set.contains(&(e[0], e[1]))))
            .map(|(e, _)| (e[0], e[1]))
            .collect()
    };
    new.sharp_edges = flags(&old.sharp_edges);
    new.seams = flags(&old.seams);
}
