// SPDX-License-Identifier: GPL-3.0-only
//! Attribute, input, geometry, instance, point, material and field-utility
//! nodes, plus the helpers the other node files share (evaluating fields on
//! a component, writing attributes, anonymous outputs).
use super::attributes::{AttrData, AttrType, Domain, Val};
use super::eval::{anonymous_name, domain_prop, Ins, NodeCx, Outs, SocketValue};
use super::field::{self, eval, konst, map, special, Cache, Ctx, Field};
use super::meshops;
use super::sample::{closest_on_segment, KdTree, Surface};
use super::topo::{write_mesh_attr, MeshTopo};
use super::{realize, Component, Geometry, InstanceRef, Instances, PointCloud};
use crate::math::{Mat4, Quat, Vec3};
use crate::mesh::Mesh;
use crate::nodes::{Prop, SocketType};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared helpers

/// Run `f` with a field context for component `c` of `g`.
pub(crate) fn with_component<R>(g: &Geometry, c: Component, f: impl FnOnce(&Ctx) -> R) -> Option<R> {
    match c {
        Component::Mesh => g.mesh.as_ref().map(|m| {
            let t = MeshTopo::new(m);
            f(&Ctx::Mesh(m, &t))
        }),
        Component::Curves => g.curves.as_ref().map(|x| f(&Ctx::Curves(x))),
        Component::Points => g.points.as_ref().map(|p| f(&Ctx::Points(p))),
        Component::Instances => g.instances.as_ref().map(|i| f(&Ctx::Instances(i))),
    }
}

pub(crate) fn ev(f: &Field, ctx: &Ctx, d: Domain, cache: &mut Cache) -> Vec<Val> {
    eval(f, ctx, d, cache).to_vec()
}

pub(crate) fn bools(v: &[Val]) -> Vec<bool> {
    v.iter().map(|x| x.b()).collect()
}

pub(crate) fn vecs(v: &[Val]) -> Vec<Vec3> {
    v.iter().map(|x| x.v()).collect()
}

pub(crate) fn floats(v: &[Val]) -> Vec<f32> {
    v.iter().map(|x| x.f()).collect()
}

pub(crate) fn ints(v: &[Val]) -> Vec<i32> {
    v.iter().map(|x| x.i()).collect()
}

/// Read a named attribute's values on `d` (None when absent).
pub(crate) fn read_attr_on(ctx: &Ctx, name: &str, d: Domain) -> Option<Vec<Val>> {
    let (src, data) = ctx.read_attr(name)?;
    Some(ctx.adapt(&data.to_vals(), src, d))
}

/// Write an attribute (built-ins included) on component `c`.
pub(crate) fn write_attr(g: &mut Geometry, c: Component, name: &str, d: Domain, data: AttrData) {
    match c {
        Component::Mesh => {
            if let Some(m) = &mut g.mesh {
                let t = MeshTopo::new(m);
                write_mesh_attr(m, &t, name, d, data);
            }
        }
        Component::Curves => {
            if let Some(x) = &mut g.curves {
                if name == "position" {
                    let v = x.adapt(&data.to_vals(), d, Domain::Point);
                    x.positions = vecs(&v);
                } else {
                    x.attributes.set(name, d, data);
                }
            }
        }
        Component::Points => {
            if let Some(p) = &mut g.points {
                if name == "position" {
                    p.positions = vecs(&data.to_vals());
                } else {
                    p.attributes.set(name, Domain::Point, data);
                }
            }
        }
        Component::Instances => {
            if let Some(i) = &mut g.instances {
                if name == "position" {
                    for (t, v) in i.transforms.iter_mut().zip(data.to_vals()) {
                        let mut m = *t;
                        m.set_col3(3, v.v());
                        *t = m;
                    }
                } else {
                    i.attributes.set(name, Domain::Instance, data);
                }
            }
        }
    }
}

/// Values of `f` on `d` where `sel` is true, the attribute's old values
/// (or `def`) elsewhere.
pub(crate) fn masked(new: Vec<Val>, old: Option<Vec<Val>>, sel: &[bool], def: Val) -> Vec<Val> {
    new.into_iter().enumerate().map(|(i, v)| if sel.get(i).copied().unwrap_or(true) { v } else { old.as_ref().and_then(|o| o.get(i).copied()).unwrap_or(def) }).collect()
}

/// The components of `g` that have domain `d`.
pub(crate) fn components_with(g: &Geometry, d: Domain) -> Vec<Component> {
    Component::ALL.into_iter().filter(|&c| g.has(c) && c.domains().contains(&d)).collect()
}

/// An output field that reads an anonymous attribute.
pub(crate) fn anon_field(name: &str, ty: AttrType) -> SocketValue {
    SocketValue::Field(field::named_attribute(name, ty))
}

/// Store per-element marks as an anonymous attribute; returns its field.
pub(crate) fn store_marks(m: &mut Mesh, d: Domain, marks: Vec<bool>) -> SocketValue {
    let name = anonymous_name();
    m.attributes.set(&name, d, AttrData::Bool(marks));
    anon_field(&name, AttrType::Bool)
}

pub(crate) fn data_type(ins: &Ins, key: &str) -> AttrType {
    AttrType::from_name(ins.node.enum_prop(key)).unwrap_or(AttrType::Float)
}

/// The geometry a sampling node reads from: the first component with `d`.
fn source_component(g: &Geometry, d: Domain) -> Option<Component> {
    let order: &[Component] = match d {
        Domain::Point => &[Component::Mesh, Component::Points, Component::Curves],
        Domain::Curve => &[Component::Curves],
        Domain::Instance => &[Component::Instances],
        _ => &[Component::Mesh],
    };
    order.iter().copied().find(|&c| g.has(c))
}

fn owner_materials(cx: &NodeCx) -> Vec<usize> {
    match (cx.opts.scene, cx.opts.self_object) {
        (Some(s), Some(o)) => s.objects.get(o).map(|o| o.materials.clone()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------

pub(crate) fn exec(cx: &NodeCx, ins: &Ins) -> Option<Vec<SocketValue>> {
    let node = ins.node;
    let mut o = Outs::new(node);
    match node.idname.as_str() {
        // Inputs ----------------------------------------------------------
        "GeometryNodeInputPosition" => {
            o.field("Position", field::position());
        }
        "GeometryNodeInputNormal" => {
            o.field("Normal", field::normal());
        }
        "GeometryNodeInputIndex" => {
            o.field("Index", field::index());
        }
        "GeometryNodeInputID" => {
            o.field("ID", field::id());
        }
        "GeometryNodeInputRadius" => {
            o.field("Radius", field::radius());
        }
        "GeometryNodeInputNamedAttribute" => {
            let name = ins.s("Name");
            let ty = data_type(ins, "data_type");
            o.field("Attribute", field::named_attribute(&name, ty));
            o.field("Exists", field::attribute_exists(&name));
        }
        "GeometryNodeInputMaterial" => {
            let id = match node.prop("material") {
                Some(Prop::Id(i)) => *i,
                _ => None,
            };
            o.set("Material", SocketValue::Id(id));
        }
        "GeometryNodeSelfObject" => {
            o.set("Self Object", SocketValue::Id(cx.opts.self_object));
        }
        "GeometryNodeIsViewport" => {
            o.val("Is Viewport", Val::Bool(true));
        }
        "GeometryNodeInputActiveCamera" => {
            o.set("Active Camera", SocketValue::Id(cx.opts.scene.and_then(|s| s.active_camera)));
        }
        "GeometryNodeInputSceneTime" => {
            let fps = cx.opts.scene.map_or(24.0, |s| s.fps).max(1e-6);
            o.val("Seconds", Val::Float(cx.opts.frame / fps));
            o.val("Frame", Val::Float(cx.opts.frame));
        }
        "GeometryNodeObjectInfo" => object_info(cx, ins, &mut o),
        "GeometryNodeCollectionInfo" => collection_info(cx, ins, &mut o),
        "GeometryNodeInputMaterialIndex" => {
            o.field("Material Index", field::named_attribute("material_index", AttrType::Int));
        }
        "GeometryNodeInputShadeSmooth" => {
            o.field("Smooth", map(vec![field::named_attribute("sharp_face", AttrType::Bool)], Val::Bool(true), |v| Val::Bool(!v[0].b())));
        }
        "GeometryNodeInputEdgeSmooth" => {
            o.field("Smooth", map(vec![field::named_attribute("sharp_edge", AttrType::Bool)], Val::Bool(true), |v| Val::Bool(!v[0].b())));
        }
        "GeometryNodeViewer" => {}

        // Attribute -------------------------------------------------------
        "GeometryNodeAttributeStatistic" => attribute_statistic(ins, &mut o),
        "GeometryNodeAttributeDomainSize" => {
            let g = ins.geo("Geometry");
            let c = Component::from_name(node.enum_prop("component")).unwrap_or(Component::Mesh);
            let names = [("Point Count", Domain::Point), ("Edge Count", Domain::Edge), ("Face Count", Domain::Face), ("Face Corner Count", Domain::Corner), ("Spline Count", Domain::Curve), ("Instance Count", Domain::Instance)];
            for (n, d) in names {
                o.val(n, Val::Int(g.domain_size(c, d) as i32));
            }
            o.val("Layer Count", Val::Int(0));
        }
        "GeometryNodeBlurAttribute" => {
            let value = ins.field("Value");
            let iters = ins.i("Iterations").max(0) as usize;
            let weight = ins.field("Weight");
            let def = value.default_val();
            o.field("Value", special("Blur", def, move |ctx, d, cache| blur(ctx, d, cache, &value, &weight, iters)));
        }
        "GeometryNodeCaptureAttribute" => {
            let mut g = ins.geo("Geometry");
            let d = domain_prop(node, "domain");
            for (k, s) in node.inputs.iter().enumerate() {
                if s.ty == SocketType::Geometry || s.ty == SocketType::Virtual {
                    continue;
                }
                let f = ins.vals.get(k).map(|v| v.field()).unwrap_or_else(|| konst(super::func::value_to_val(&s.value, s.ty)));
                let ty = AttrType::of_val(&f.default_val());
                let name = anonymous_name();
                for c in components_with(&g, d) {
                    let vals = with_component(&g, c, |ctx| ev(&f, ctx, d, &mut Cache::default())).unwrap_or_default();
                    write_attr(&mut g, c, &name, d, AttrData::from_vals(ty, &vals));
                }
                o.set(&s.identifier, anon_field(&name, ty));
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeRemoveAttribute" => {
            let mut g = ins.geo("Geometry");
            let name = ins.s("Name");
            let wild = node.enum_prop("pattern_mode") == "WILDCARD";
            let matches = |n: &str| if wild { wildcard(&name, n) } else { n == name };
            if let Some(m) = &mut g.mesh {
                m.attributes.list.retain(|a| !matches(&a.name));
                if let Some(gi) = m.vertex_groups.iter().position(|n| matches(n)) {
                    m.remove_vertex_group(gi);
                }
            }
            if let Some(c) = &mut g.curves {
                c.attributes.list.retain(|a| !matches(&a.name));
            }
            if let Some(p) = &mut g.points {
                p.attributes.list.retain(|a| !matches(&a.name));
            }
            if let Some(i) = &mut g.instances {
                i.attributes.list.retain(|a| !matches(&a.name));
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeStoreNamedAttribute" => {
            let mut g = ins.geo("Geometry");
            let name = ins.s("Name");
            let d = domain_prop(node, "domain");
            let ty = data_type(ins, "data_type");
            let f = ins.field("Value");
            let sel = ins.field("Selection");
            if !name.is_empty() {
                for c in components_with(&g, d) {
                    let vals = with_component(&g, c, |ctx| {
                        let mut cache = Cache::default();
                        let s = bools(&ev(&sel, ctx, d, &mut cache));
                        let new: Vec<Val> = ev(&f, ctx, d, &mut cache).into_iter().map(|v| ty.convert(v)).collect();
                        let old = read_attr_on(ctx, &name, d);
                        masked(new, old, &s, ty.default_val())
                    });
                    if let Some(v) = vals {
                        write_attr(&mut g, c, &name, d, AttrData::from_vals(ty, &v));
                    }
                }
            }
            o.geo("Geometry", g);
        }

        // Geometry --------------------------------------------------------
        "GeometryNodeJoinGeometry" => {
            o.geo("Geometry", ins.geo("Geometry"));
        }
        "GeometryNodeBoundBox" => {
            let g = ins.geo("Geometry");
            let b = g.bounds(&|r| cx.opts.resolve(r));
            if b.is_empty() {
                o.geo("Bounding Box", Geometry::default());
                o.val("Min", Val::Vector(Vec3::ZERO)).val("Max", Val::Vector(Vec3::ZERO));
            } else {
                let mut m = super::primitives::cube(b.max - b.min, [2, 2, 2]);
                m.translate((b.min + b.max) * 0.5);
                o.geo("Bounding Box", Geometry::from_mesh(m));
                o.val("Min", Val::Vector(b.min)).val("Max", Val::Vector(b.max));
            }
        }
        "GeometryNodeConvexHull" => {
            let g = realize(&ins.geo("Geometry"), &|r| cx.opts.resolve(r));
            let pts: Vec<Vec3> = g.mesh.iter().flat_map(|m| m.positions.iter()).chain(g.curves.iter().flat_map(|c| c.positions.iter())).chain(g.points.iter().flat_map(|p| p.positions.iter())).copied().collect();
            let m = match crate::ops2::convex_hull_triangles(&pts) {
                Some(tris) => {
                    let mut used: Vec<usize> = tris.iter().flatten().copied().collect();
                    used.sort_unstable();
                    used.dedup();
                    let mut remap = vec![0u32; pts.len()];
                    for (k, &i) in used.iter().enumerate() {
                        remap[i] = k as u32;
                    }
                    Mesh::from_faces(used.iter().map(|&i| pts[i]).collect(), tris.iter().map(|t| t.iter().map(|&i| remap[i]).collect()).collect())
                }
                None => Mesh { positions: pts, ..Default::default() },
            };
            o.geo("Convex Hull", Geometry::from_mesh(m));
        }
        "GeometryNodeDeleteGeometry" | "GeometryNodeSeparateGeometry" => {
            let g = ins.geo("Geometry");
            let d = domain_prop(node, "domain");
            let sel = ins.field("Selection");
            let mode = if node.idname == "GeometryNodeDeleteGeometry" { node.enum_prop("mode").to_string() } else { "ALL".to_string() };
            if node.idname == "GeometryNodeDeleteGeometry" {
                o.geo("Geometry", delete_geometry(&g, d, &sel, false, &mode));
            } else {
                o.geo("Selection", delete_geometry(&g, d, &sel, true, &mode));
                o.geo("Inverted", delete_geometry(&g, d, &sel, false, &mode));
            }
        }
        "GeometryNodeDuplicateElements" => duplicate_elements(ins, &mut o),
        "GeometryNodeMergeByDistance" => {
            let mut g = ins.geo("Geometry");
            let dist = ins.f("Distance").max(0.0);
            let sel = ins.field("Selection");
            let connected = node.enum_prop("mode") == "CONNECTED";
            if let Some(m) = &g.mesh {
                let t = MeshTopo::new(m);
                let s = bools(&ev(&sel, &Ctx::Mesh(m, &t), Domain::Point, &mut Cache::default()));
                g.mesh = Some(meshops::merge_by_distance(m, &s, dist, connected));
            }
            if let Some(p) = &g.points {
                let s = bools(&ev(&sel, &Ctx::Points(p), Domain::Point, &mut Cache::default()));
                let m = Mesh { positions: p.positions.clone(), ..Default::default() };
                let merged = meshops::merge_by_distance(&m, &s, dist, false);
                // Keep the points whose positions survived.
                let kd = KdTree::new(&merged.positions);
                let mut keep = Vec::new();
                let mut taken = vec![false; merged.positions.len()];
                for (i, &q) in p.positions.iter().enumerate() {
                    if let Some((k, d2)) = kd.nearest(q) {
                        if d2 == 0.0 && !taken[k] {
                            taken[k] = true;
                            keep.push(i);
                        }
                    }
                }
                let mut np = PointCloud { positions: keep.iter().map(|&i| p.positions[i]).collect(), attributes: p.attributes.clone() };
                np.attributes.gather_domain(Domain::Point, &keep);
                g.points = Some(np);
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeSortElements" => {
            let mut g = ins.geo("Geometry");
            let d = domain_prop(node, "domain");
            sort_elements(&mut g, d, &ins.field("Selection"), &ins.field("Group ID"), &ins.field("Sort Key"));
            o.geo("Geometry", g);
        }
        "GeometryNodeTransform" => {
            let mut g = ins.geo("Geometry");
            let m = if node.enum_prop("mode") == "MATRIX" {
                ins.val("Transform").m()
            } else {
                Mat4::from_trs(ins.v("Translation"), ins.val("Rotation").q(), ins.v("Scale"))
            };
            g.transform(&m);
            o.geo("Geometry", g);
        }
        "GeometryNodeSeparateComponents" => {
            let g = ins.geo("Geometry");
            o.geo("Mesh", g.only(Component::Mesh));
            o.geo("Curve", g.only(Component::Curves));
            o.geo("Grease Pencil", Geometry::default());
            o.geo("Point Cloud", g.only(Component::Points));
            o.geo("Volume", Geometry { volume: g.volume, ..Default::default() });
            o.geo("Instances", g.only(Component::Instances));
        }
        "GeometryNodeGeometryToInstance" => {
            let mut inst = Instances::default();
            for v in ins.list("Geometry") {
                let g = v.geometry();
                let h = inst.add_reference(InstanceRef::Geometry(Box::new(g)));
                inst.add(h, Mat4::IDENTITY);
            }
            o.geo("Instances", Geometry::from_instances(inst));
        }
        "GeometryNodeSetPosition" => {
            let mut g = ins.geo("Geometry");
            let sel = ins.field("Selection");
            let pos = ins.field("Position");
            let off = ins.field("Offset");
            for c in [Component::Mesh, Component::Curves, Component::Points, Component::Instances] {
                if !g.has(c) {
                    continue;
                }
                let d = if c == Component::Instances { Domain::Instance } else { Domain::Point };
                let new = with_component(&g, c, |ctx| {
                    let mut cache = Cache::default();
                    let s = bools(&ev(&sel, ctx, d, &mut cache));
                    let p = ev(&pos, ctx, d, &mut cache);
                    let of = ev(&off, ctx, d, &mut cache);
                    let old = vecs(&ev(&field::position(), ctx, d, &mut cache));
                    old.iter().enumerate().map(|(i, &q)| if s[i] { p[i].v() + of[i].v() } else { q }).collect::<Vec<Vec3>>()
                });
                let Some(new) = new else { continue };
                if let (Component::Curves, Some(cv)) = (c, &mut g.curves) {
                    // Handles move with their points.
                    let delta: Vec<Vec3> = new.iter().zip(&cv.positions).map(|(a, b)| *a - *b).collect();
                    for name in ["handle_left", "handle_right"] {
                        if let Some(a) = cv.attributes.get_mut(name) {
                            if a.data.len() == delta.len() {
                                for (i, dv) in delta.iter().enumerate() {
                                    let v = a.data.get(i).v();
                                    a.data.set(i, Val::Vector(v + *dv));
                                }
                            }
                        }
                    }
                }
                write_attr(&mut g, c, "position", d, AttrData::Float3(new));
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeSetID" => {
            let mut g = ins.geo("Geometry");
            let sel = ins.field("Selection");
            let id = ins.field("ID");
            for c in Component::ALL {
                if !g.has(c) {
                    continue;
                }
                let d = if c == Component::Instances { Domain::Instance } else { Domain::Point };
                let v = with_component(&g, c, |ctx| {
                    let mut cache = Cache::default();
                    let s = bools(&ev(&sel, ctx, d, &mut cache));
                    let new = ev(&id, ctx, d, &mut cache).into_iter().map(|v| Val::Int(v.i())).collect();
                    let old = ev(&field::id(), ctx, d, &mut cache);
                    masked(new, Some(old), &s, Val::Int(0))
                });
                if let Some(v) = v {
                    write_attr(&mut g, c, "id", d, AttrData::from_vals(AttrType::Int, &v));
                }
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeSampleIndex" => {
            let g = ins.geo("Geometry");
            let d = domain_prop(node, "domain");
            let value = ins.field("Value");
            let def = value.default_val();
            let clamp = node.bool_prop("clamp");
            let src: Vec<Val> = source_component(&g, d).and_then(|c| with_component(&g, c, |ctx| ev(&value, ctx, d, &mut Cache::default()))).unwrap_or_default();
            let index = ins.field("Index");
            if index.is_const() {
                let i = index.default_val().i();
                o.val("Value", pick(&src, i, clamp, def));
            } else {
                let src = Arc::new(src);
                o.field("Value", special("Sample Index", def, move |ctx, dd, cache| eval(&index, ctx, dd, cache).iter().map(|i| pick(&src, i.i(), clamp, def)).collect()));
            }
        }
        "GeometryNodeSampleNearest" => {
            let g = ins.geo("Geometry");
            let d = domain_prop(node, "domain");
            let pos = ins.field("Sample Position");
            let finder = Arc::new(Nearest::new(&g, d));
            o.field("Index", special("Sample Nearest", Val::Int(0), move |ctx, dd, cache| eval(&pos, ctx, dd, cache).iter().map(|p| Val::Int(finder.find(p.v()).map_or(0, |i| i as i32))).collect()));
        }
        "GeometryNodeSampleNearestSurface" => {
            let g = ins.geo("Mesh");
            let value = ins.field("Value");
            let def = value.default_val();
            let pos = ins.field("Sample Position");
            let mesh = Arc::new(g.mesh.clone().unwrap_or_default());
            let vals = Arc::new(with_component(&g, Component::Mesh, |ctx| ev(&value, ctx, Domain::Corner, &mut Cache::default())).unwrap_or_default());
            let m2 = mesh.clone();
            let p2 = pos.clone();
            o.field("Value", special("Sample Nearest Surface", def, move |ctx, dd, cache| {
                let s = Surface::new(&mesh);
                eval(&pos, ctx, dd, cache).iter().map(|p| s.nearest(p.v()).map_or(def, |h| s.interpolate(&vals, Domain::Corner, &h))).collect()
            }));
            o.field("Is Valid", special("Sample Nearest Surface Valid", Val::Bool(false), move |ctx, dd, cache| {
                let ok = !m2.faces.is_empty();
                vec![Val::Bool(ok); eval(&p2, ctx, dd, cache).len()]
            }));
        }
        "GeometryNodeRaycast" => raycast(ins, &mut o),
        "GeometryNodeProximity" => {
            let g = ins.geo("Target");
            let target = node.enum_prop("target_element").to_string();
            let pos = ins.field("Source Position");
            let prox = Arc::new(Proximity::new(&g, &target));
            let (p1, p2, p3) = (prox.clone(), prox.clone(), prox);
            let (f1, f2, f3) = (pos.clone(), pos.clone(), pos);
            o.field("Position", special("Proximity", Val::Vector(Vec3::ZERO), move |ctx, d, c| eval(&f1, ctx, d, c).iter().map(|p| Val::Vector(p1.query(p.v()).map_or(Vec3::ZERO, |x| x.0))).collect()));
            o.field("Distance", special("Proximity Distance", Val::Float(0.0), move |ctx, d, c| eval(&f2, ctx, d, c).iter().map(|p| Val::Float(p2.query(p.v()).map_or(0.0, |x| x.1))).collect()));
            o.field("Is Valid", special("Proximity Valid", Val::Bool(false), move |ctx, d, c| eval(&f3, ctx, d, c).iter().map(|p| Val::Bool(p3.query(p.v()).is_some())).collect()));
        }

        // Instances -------------------------------------------------------
        "GeometryNodeInstanceOnPoints" => instance_on_points(ins, &mut o),
        "GeometryNodeRealizeInstances" => {
            let g = ins.geo("Geometry");
            let sel = ins.field("Selection");
            let resolve = |r: &InstanceRef| cx.opts.resolve(r);
            let out = match &g.instances {
                Some(inst) if !sel.is_const() || !sel.default_val().b() => {
                    let s = bools(&ev(&sel, &Ctx::Instances(inst), Domain::Instance, &mut Cache::default()));
                    let keep: Vec<usize> = (0..inst.len()).filter(|&i| !s[i]).collect();
                    let take: Vec<usize> = (0..inst.len()).filter(|&i| s[i]).collect();
                    let mut part = Geometry { instances: Some(inst.gather(&take)), ..g.clone() };
                    part = realize(&part, &resolve);
                    if !keep.is_empty() {
                        part.instances = Some(inst.gather(&keep));
                    }
                    part
                }
                _ => realize(&g, &resolve),
            };
            o.geo("Geometry", out);
        }
        "GeometryNodeTranslateInstances" | "GeometryNodeRotateInstances" | "GeometryNodeScaleInstances" => {
            let mut g = ins.geo("Instances");
            if let Some(inst) = &g.instances {
                let ctx = Ctx::Instances(inst);
                let mut cache = Cache::default();
                let n = inst.len();
                let s = bools(&ev(&ins.field("Selection"), &ctx, Domain::Instance, &mut cache));
                let local = bools(&ev(&ins.field("Local Space"), &ctx, Domain::Instance, &mut cache));
                let mut new = inst.transforms.clone();
                match node.idname.as_str() {
                    "GeometryNodeTranslateInstances" => {
                        let t = ev(&ins.field("Translation"), &ctx, Domain::Instance, &mut cache);
                        for i in (0..n).filter(|&i| s[i]) {
                            let m = Mat4::translation(t[i].v());
                            new[i] = if local[i] { new[i] * m } else { m * new[i] };
                        }
                    }
                    "GeometryNodeRotateInstances" => {
                        let r = ev(&ins.field("Rotation"), &ctx, Domain::Instance, &mut cache);
                        let p = ev(&ins.field("Pivot Point"), &ctx, Domain::Instance, &mut cache);
                        for i in (0..n).filter(|&i| s[i]) {
                            let rot = Mat4::from_trs(Vec3::ZERO, r[i].q(), Vec3::ONE);
                            let piv = p[i].v();
                            let about = Mat4::translation(piv) * rot * Mat4::translation(-piv);
                            new[i] = if local[i] {
                                new[i] * about
                            } else {
                                let t = new[i].translation_part();
                                Mat4::translation(t) * about * Mat4::translation(-t) * new[i]
                            };
                        }
                    }
                    _ => {
                        let sc = ev(&ins.field("Scale"), &ctx, Domain::Instance, &mut cache);
                        let ce = ev(&ins.field("Center"), &ctx, Domain::Instance, &mut cache);
                        for i in (0..n).filter(|&i| s[i]) {
                            let sm = Mat4::from_trs(Vec3::ZERO, Quat::IDENTITY, sc[i].v());
                            let c = ce[i].v();
                            let about = Mat4::translation(c) * sm * Mat4::translation(-c);
                            new[i] = if local[i] { new[i] * about } else { about * new[i] };
                        }
                    }
                }
                if let Some(inst) = &mut g.instances {
                    inst.transforms = new;
                }
            }
            o.geo("Instances", g);
        }
        "GeometryNodeInstancesToPoints" => {
            let g = ins.geo("Instances");
            let mut pc = PointCloud::default();
            if let Some(inst) = &g.instances {
                let ctx = Ctx::Instances(inst);
                let mut cache = Cache::default();
                let s = bools(&ev(&ins.field("Selection"), &ctx, Domain::Instance, &mut cache));
                let p = vecs(&ev(&ins.field("Position"), &ctx, Domain::Instance, &mut cache));
                let r = floats(&ev(&ins.field("Radius"), &ctx, Domain::Instance, &mut cache));
                let keep: Vec<usize> = (0..inst.len()).filter(|&i| s[i]).collect();
                pc.positions = keep.iter().map(|&i| p[i]).collect();
                for a in &inst.attributes.list {
                    if a.data.len() == inst.len() {
                        pc.attributes.set(&a.name, Domain::Point, a.data.gather(&keep));
                    }
                }
                pc.attributes.set("radius", Domain::Point, AttrData::Float(keep.iter().map(|&i| r[i]).collect()));
            }
            o.geo("Points", Geometry::from_points(pc));
        }
        "GeometryNodeInputInstanceRotation" => {
            o.field("Rotation", instance_input("Instance Rotation", Val::Rotation(Quat::IDENTITY), |m| Val::Rotation(m.to_trs().1)));
        }
        "GeometryNodeInputInstanceScale" => {
            o.field("Scale", instance_input("Instance Scale", Val::Vector(Vec3::ONE), |m| Val::Vector(m.to_trs().2)));
        }
        "GeometryNodeInstanceTransform" => {
            o.field("Transform", instance_input("Instance Transform", Val::Matrix(Mat4::IDENTITY), Val::Matrix));
        }

        // Point -----------------------------------------------------------
        "GeometryNodePoints" => {
            let count = ins.i("Count").max(0) as usize;
            let mut pc = PointCloud { positions: vec![Vec3::ZERO; count], ..Default::default() };
            let ctx = Ctx::Points(&pc);
            let mut cache = Cache::default();
            let p = vecs(&ev(&ins.field("Position"), &ctx, Domain::Point, &mut cache));
            let r = floats(&ev(&ins.field("Radius"), &ctx, Domain::Point, &mut cache));
            pc.positions = p;
            pc.attributes.set("radius", Domain::Point, AttrData::Float(r));
            o.geo("Geometry", Geometry::from_points(pc));
        }
        "GeometryNodeSetPointRadius" => {
            let mut g = ins.geo("Points");
            if let Some(p) = &g.points {
                let ctx = Ctx::Points(p);
                let mut cache = Cache::default();
                let s = bools(&ev(&ins.field("Selection"), &ctx, Domain::Point, &mut cache));
                let r = ev(&ins.field("Radius"), &ctx, Domain::Point, &mut cache);
                let old: Vec<Val> = p.radius().into_iter().map(Val::Float).collect();
                let v = masked(r, Some(old), &s, Val::Float(0.01));
                write_attr(&mut g, Component::Points, "radius", Domain::Point, AttrData::from_vals(AttrType::Float, &v));
            }
            o.geo("Points", g);
        }
        "GeometryNodePointsToVertices" => {
            let g = ins.geo("Points");
            let mut out = Geometry::default();
            if let Some(p) = &g.points {
                let s = bools(&ev(&ins.field("Selection"), &Ctx::Points(p), Domain::Point, &mut Cache::default()));
                let keep: Vec<usize> = (0..p.positions.len()).filter(|&i| s[i]).collect();
                let mut m = Mesh { positions: keep.iter().map(|&i| p.positions[i]).collect(), ..Default::default() };
                for a in &p.attributes.list {
                    if a.name != "radius" && a.data.len() == p.positions.len() {
                        m.attributes.set(&a.name, Domain::Point, a.data.gather(&keep));
                    }
                }
                out.mesh = Some(m);
            }
            o.geo("Mesh", out);
        }

        // Material --------------------------------------------------------
        "GeometryNodeSetMaterialIndex" => {
            let mut g = ins.geo("Geometry");
            if let Some(m) = &g.mesh {
                let t = MeshTopo::new(m);
                let ctx = Ctx::Mesh(m, &t);
                let mut cache = Cache::default();
                let s = bools(&ev(&ins.field("Selection"), &ctx, Domain::Face, &mut cache));
                let v = ev(&ins.field("Material Index"), &ctx, Domain::Face, &mut cache);
                let old = read_attr_on(&ctx, "material_index", Domain::Face);
                let v = masked(v, old, &s, Val::Int(0));
                write_attr(&mut g, Component::Mesh, "material_index", Domain::Face, AttrData::from_vals(AttrType::Int, &v));
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeSetMaterial" => {
            let mut g = ins.geo("Geometry");
            let mats = owner_materials(cx);
            let slot = ins.id("Material").and_then(|id| mats.iter().position(|&x| x == id)).unwrap_or(0) as i32;
            if let Some(m) = &g.mesh {
                let t = MeshTopo::new(m);
                let ctx = Ctx::Mesh(m, &t);
                let s = bools(&ev(&ins.field("Selection"), &ctx, Domain::Face, &mut Cache::default()));
                if let Some(m) = &mut g.mesh {
                    for (f, face) in m.faces.iter_mut().enumerate() {
                        if s[f] {
                            face.material = slot as u16;
                        }
                    }
                }
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeReplaceMaterial" => {
            let mut g = ins.geo("Geometry");
            let mats = owner_materials(cx);
            let (old, new) = (ins.id("Old"), ins.id("New"));
            let slot_of = |id: Option<usize>| id.and_then(|id| mats.iter().position(|&x| x == id));
            if let (Some(m), Some(os)) = (&mut g.mesh, slot_of(old)) {
                let ns = slot_of(new).unwrap_or(0) as u16;
                for f in &mut m.faces {
                    if f.material as usize == os {
                        f.material = ns;
                    }
                }
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeMaterialSelection" => {
            let mats = owner_materials(cx);
            let want = ins.id("Material");
            o.field("Selection", field::input("Material Selection", Val::Bool(false), move |ctx, _| match ctx {
                Ctx::Mesh(m, _) => Some((Domain::Face, m.faces.iter().map(|f| Val::Bool(want.is_some() && mats.get(f.material as usize).copied() == want)).collect())),
                _ => None,
            }));
        }

        // Field utilities -------------------------------------------------
        "GeometryNodeAccumulateField" => {
            let d = domain_prop(node, "domain");
            let value = ins.field("Value");
            let group = ins.field("Group ID");
            let def = value.default_val();
            for (k, out) in ["Leading", "Trailing", "Total"].iter().enumerate() {
                let (v, g) = (value.clone(), group.clone());
                o.field(out, special("Accumulate", def, move |ctx, dd, cache| {
                    let vals = ev(&v, ctx, d, cache);
                    let groups = ints(&ev(&g, ctx, d, cache));
                    let res = accumulate(&vals, &groups, k);
                    ctx.adapt(&res, d, dd)
                }));
            }
        }
        "GeometryNodeFieldAtIndex" => {
            let d = domain_prop(node, "domain");
            let value = ins.field("Value");
            let index = ins.field("Index");
            let def = value.default_val();
            o.field("Value", special("Evaluate at Index", def, move |ctx, dd, cache| {
                let vals = ev(&value, ctx, d, cache);
                eval(&index, ctx, dd, cache).iter().map(|i| pick(&vals, i.i(), false, def)).collect()
            }));
        }
        "GeometryNodeFieldOnDomain" => {
            let d = domain_prop(node, "domain");
            let value = ins.field("Value");
            if value.is_const() {
                o.set("Value", SocketValue::Field(value));
            } else {
                let def = value.default_val();
                o.field("Value", special("Evaluate on Domain", def, move |ctx, dd, cache| {
                    let vals = ev(&value, ctx, d, cache);
                    ctx.adapt(&vals, d, dd)
                }));
            }
        }
        _ => return None,
    }
    Some(o.done())
}

fn wildcard(pattern: &str, s: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == s,
        Some((a, b)) => s.len() >= a.len() + b.len() && s.starts_with(a) && s.ends_with(b),
    }
}

fn pick(vals: &[Val], i: i32, clamp: bool, def: Val) -> Val {
    if vals.is_empty() {
        return def;
    }
    let i = if clamp { i.clamp(0, vals.len() as i32 - 1) } else { i };
    if i < 0 {
        return def;
    }
    vals.get(i as usize).copied().unwrap_or(def)
}

/// Running sums per group: 0 leading (inclusive), 1 trailing (exclusive),
/// 2 total.
fn accumulate(vals: &[Val], groups: &[i32], which: usize) -> Vec<Val> {
    use std::collections::HashMap;
    let add = |a: Val, b: Val| match (a, b) {
        (Val::Int(x), Val::Int(y)) => Val::Int(x.wrapping_add(y)),
        (Val::Vector(x), y) => Val::Vector(x + y.v()),
        (Val::Matrix(x), y) => Val::Matrix(x * y.m()),
        (Val::Rotation(x), y) => Val::Rotation(y.q() * x),
        (x, y) => Val::Float(x.f() + y.f()),
    };
    let zero = vals.first().map(|v| match v {
        Val::Int(_) => Val::Int(0),
        Val::Vector(_) => Val::Vector(Vec3::ZERO),
        Val::Matrix(_) => Val::Matrix(Mat4::IDENTITY),
        Val::Rotation(_) => Val::Rotation(Quat::IDENTITY),
        _ => Val::Float(0.0),
    });
    let Some(zero) = zero else { return Vec::new() };
    let mut run: HashMap<i32, Val> = HashMap::new();
    let mut out = Vec::with_capacity(vals.len());
    for (i, &v) in vals.iter().enumerate() {
        let g = groups.get(i).copied().unwrap_or(0);
        let acc = run.entry(g).or_insert(zero);
        let before = *acc;
        *acc = add(*acc, v);
        out.push(if which == 1 { before } else { *acc });
    }
    if which == 2 {
        for (i, o) in out.iter_mut().enumerate() {
            *o = run[&groups.get(i).copied().unwrap_or(0)];
        }
    }
    out
}

fn instance_input(name: &str, def: Val, f: fn(Mat4) -> Val) -> Field {
    field::input(name, def, move |ctx, _| match ctx {
        Ctx::Instances(i) => Some((Domain::Instance, i.transforms.iter().map(|&m| f(m)).collect())),
        _ => None,
    })
}

fn object_info(cx: &NodeCx, ins: &Ins, o: &mut Outs) {
    let Some(s) = cx.opts.scene else { return };
    let Some(i) = ins.id("Object").filter(|&i| i < s.objects.len()) else {
        o.val("Transform", Val::Matrix(Mat4::IDENTITY)).val("Scale", Val::Vector(Vec3::ONE));
        return;
    };
    let relative = ins.node.enum_prop("transform_space") == "RELATIVE";
    let world = s.world_matrix(i);
    let self_world = cx.opts.self_object.map_or(Mat4::IDENTITY, |o| s.world_matrix(o));
    let m = if relative { self_world.inverse() * world } else { world };
    let (t, r, sc) = m.to_trs();
    o.val("Transform", Val::Matrix(m)).val("Location", Val::Vector(t)).val("Rotation", Val::Rotation(r)).val("Scale", Val::Vector(sc));
    if Some(i) == cx.opts.self_object {
        // An object cannot read its own evaluated geometry.
        o.geo("Geometry", Geometry::default());
        return;
    }
    let g = if ins.b("As Instance") {
        let mut inst = Instances::default();
        let h = inst.add_reference(InstanceRef::Object(i));
        inst.add(h, if relative { m } else { Mat4::IDENTITY });
        Geometry::from_instances(inst)
    } else {
        let mut g = super::eval::object_geometry(s, i, cx.opts.self_object, cx.opts.depth).unwrap_or_default();
        if relative {
            g.transform(&m);
        }
        g
    };
    o.geo("Geometry", g);
}

fn collection_info(cx: &NodeCx, ins: &Ins, o: &mut Outs) {
    let Some(s) = cx.opts.scene else { return };
    let Some(c) = ins.id("Collection") else { return };
    let relative = ins.node.enum_prop("transform_space") == "RELATIVE";
    let self_inv = cx.opts.self_object.map_or(Mat4::IDENTITY, |o| s.world_matrix(o).inverse());
    let mut inst = Instances::default();
    if ins.b("Separate Children") {
        let reset = ins.b("Reset Children");
        for (i, ob) in s.objects.iter().enumerate() {
            if ob.collection != c || ob.parent.is_some_and(|p| s.objects[p].collection == c) || Some(i) == cx.opts.self_object {
                continue;
            }
            let h = inst.add_reference(InstanceRef::Object(i));
            let w = if reset { Mat4::IDENTITY } else { s.world_matrix(i) };
            inst.add(h, if relative { self_inv * w } else { w });
        }
    } else {
        let h = inst.add_reference(InstanceRef::Collection(c));
        inst.add(h, if relative { self_inv } else { Mat4::IDENTITY });
    }
    o.geo("Instances", Geometry::from_instances(inst));
}

fn attribute_statistic(ins: &Ins, o: &mut Outs) {
    let g = ins.geo("Geometry");
    let d = domain_prop(ins.node, "domain");
    let vector = ins.node.enum_prop("data_type") == "FLOAT_VECTOR";
    let f = if vector { ins.field("Attribute_001") } else { ins.field("Attribute") };
    let sel = ins.field("Selection");
    let mut vals: Vec<Val> = Vec::new();
    for c in components_with(&g, d) {
        if let Some(v) = with_component(&g, c, |ctx| {
            let mut cache = Cache::default();
            let s = bools(&ev(&sel, ctx, d, &mut cache));
            ev(&f, ctx, d, &mut cache).into_iter().enumerate().filter(|(i, _)| s[*i]).map(|x| x.1).collect::<Vec<Val>>()
        }) {
            vals.extend(v);
        }
    }
    let suffix = if vector { "_001" } else { "" };
    let stats: Vec<[f32; 3]> = if vals.is_empty() {
        vec![[0.0; 3]; 8]
    } else {
        let comps: Vec<Vec<f32>> = (0..if vector { 3 } else { 1 }).map(|k| vals.iter().map(|v| if vector { v.v()[k] } else { v.f() }).collect()).collect();
        let per: Vec<[f32; 8]> = comps.iter().map(|c| stats_of(c)).collect();
        (0..8).map(|s| [per[0][s], per.get(1).map_or(0.0, |p| p[s]), per.get(2).map_or(0.0, |p| p[s])]).collect()
    };
    let names = ["Mean", "Median", "Sum", "Min", "Max", "Range", "Standard Deviation", "Variance"];
    for (k, n) in names.iter().enumerate() {
        let id = format!("{n}{suffix}");
        let v = if vector { Val::Vector(Vec3::new(stats[k][0], stats[k][1], stats[k][2])) } else { Val::Float(stats[k][0]) };
        o.val(&id, v);
    }
}

fn stats_of(v: &[f32]) -> [f32; 8] {
    let n = v.len() as f32;
    let sum: f32 = v.iter().sum();
    let mean = sum / n;
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let median = if s.len() % 2 == 1 { s[s.len() / 2] } else { (s[s.len() / 2 - 1] + s[s.len() / 2]) * 0.5 };
    let (min, max) = (s[0], s[s.len() - 1]);
    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / n;
    [mean, median, sum, min, max, max - min, var.sqrt(), var]
}

/// Neighbour averaging, `iters` times (Blender's Blur Attribute).
fn blur(ctx: &Ctx, d: Domain, cache: &mut Cache, value: &Field, weight: &Field, iters: usize) -> Vec<Val> {
    let mut vals = ev(value, ctx, d, cache);
    let w = floats(&ev(weight, ctx, d, cache));
    let n = vals.len();
    let neighbors: Vec<Vec<usize>> = match ctx {
        Ctx::Mesh(m, t) => match d {
            Domain::Point => (0..n).map(|v| t.vert_edges[v].iter().map(|&e| {
                let ed = t.edges[e as usize];
                if ed[0] as usize == v { ed[1] as usize } else { ed[0] as usize }
            }).collect()).collect(),
            Domain::Edge => (0..n).map(|e| {
                let ed = t.edges[e];
                ed.iter().flat_map(|&v| t.vert_edges[v as usize].iter().map(|&x| x as usize)).filter(|&x| x != e).collect()
            }).collect(),
            Domain::Face => (0..m.faces.len()).map(|f| t.corners(f).flat_map(|c| t.edge_faces[t.corner_edge[c] as usize].iter().map(|&x| x as usize)).filter(|&x| x != f).collect()).collect(),
            _ => vec![Vec::new(); n],
        },
        Ctx::Curves(c) if d == Domain::Point => {
            let mut nb = vec![Vec::new(); n];
            for ci in 0..c.curves_num() {
                let r = c.points(ci);
                let cyc = c.cyclic(ci);
                for i in r.clone() {
                    if i > r.start {
                        nb[i].push(i - 1);
                    } else if cyc && r.len() > 2 {
                        nb[i].push(r.end - 1);
                    }
                    if i + 1 < r.end {
                        nb[i].push(i + 1);
                    } else if cyc && r.len() > 2 {
                        nb[i].push(r.start);
                    }
                }
            }
            nb
        }
        _ => vec![Vec::new(); n],
    };
    if vals.is_empty() {
        return vals;
    }
    let ty = AttrType::of_val(&vals[0]);
    for _ in 0..iters {
        let prev = vals.clone();
        for i in 0..n {
            let mut items: Vec<(Val, f32)> = vec![(prev[i], 1.0)];
            for &j in &neighbors[i] {
                items.push((prev[j], w.get(j).copied().unwrap_or(1.0)));
            }
            let total: f32 = items.iter().map(|x| x.1).sum();
            if total > 0.0 {
                for it in &mut items {
                    it.1 /= total;
                }
                vals[i] = ty.convert(super::attributes::interp(&items));
            }
        }
    }
    vals
}

/// Delete (or keep, with `keep_selected`) the selected elements of every
/// component that has domain `d`.
pub(crate) fn delete_geometry(g: &Geometry, d: Domain, sel: &Field, keep_selected: bool, mode: &str) -> Geometry {
    let mut out = g.clone();
    let flip = |v: Vec<bool>| -> Vec<bool> { if keep_selected { v.into_iter().map(|b| !b).collect() } else { v } };
    if let Some(m) = &g.mesh {
        if Component::Mesh.domains().contains(&d) {
            let t = MeshTopo::new(m);
            let s = flip(bools(&ev(sel, &Ctx::Mesh(m, &t), d, &mut Cache::default())));
            out.mesh = Some(meshops::delete(m, d, &s, mode));
        }
    }
    if let Some(c) = &g.curves {
        if d == Domain::Point || d == Domain::Curve {
            let s = flip(bools(&ev(sel, &Ctx::Curves(c), d, &mut Cache::default())));
            out.curves = Some(if d == Domain::Curve {
                let keep: Vec<usize> = (0..c.curves_num()).filter(|&i| !s[i]).collect();
                c.gather_curves(&keep)
            } else {
                let keep: Vec<Vec<usize>> = (0..c.curves_num()).map(|ci| c.points(ci).filter(|&i| !s[i]).collect()).collect();
                c.gather_points(&keep)
            });
        }
    }
    if let Some(p) = &g.points {
        if d == Domain::Point {
            let s = flip(bools(&ev(sel, &Ctx::Points(p), d, &mut Cache::default())));
            let keep: Vec<usize> = (0..p.positions.len()).filter(|&i| !s[i]).collect();
            let mut np = PointCloud { positions: keep.iter().map(|&i| p.positions[i]).collect(), attributes: p.attributes.clone() };
            np.attributes.gather_domain(Domain::Point, &keep);
            out.points = Some(np);
        }
    }
    if let Some(i) = &g.instances {
        if d == Domain::Instance {
            let s = flip(bools(&ev(sel, &Ctx::Instances(i), d, &mut Cache::default())));
            let keep: Vec<usize> = (0..i.len()).filter(|&k| !s[k]).collect();
            out.instances = Some(i.gather(&keep));
        }
    }
    out
}

fn duplicate_elements(ins: &Ins, o: &mut Outs) {
    let g = ins.geo("Geometry");
    let d = domain_prop(ins.node, "domain");
    let sel = ins.field("Selection");
    let amount = ins.field("Amount");
    let mut out = Geometry::default();
    let idx_name = anonymous_name();
    match d {
        Domain::Instance => {
            if let Some(inst) = &g.instances {
                let ctx = Ctx::Instances(inst);
                let mut cache = Cache::default();
                let s = bools(&ev(&sel, &ctx, d, &mut cache));
                let a = ints(&ev(&amount, &ctx, d, &mut cache));
                let mut map = Vec::new();
                let mut idx = Vec::new();
                for i in 0..inst.len() {
                    if s[i] {
                        for k in 0..a[i].max(0) {
                            map.push(i);
                            idx.push(k);
                        }
                    }
                }
                let mut ni = inst.gather(&map);
                ni.attributes.set(&idx_name, Domain::Instance, AttrData::Int(idx));
                out.instances = Some(ni);
            }
        }
        Domain::Curve => {
            if let Some(c) = &g.curves {
                let ctx = Ctx::Curves(c);
                let mut cache = Cache::default();
                let s = bools(&ev(&sel, &ctx, d, &mut cache));
                let a = ints(&ev(&amount, &ctx, d, &mut cache));
                let mut map = Vec::new();
                let mut idx = Vec::new();
                for i in 0..c.curves_num() {
                    if s[i] {
                        for k in 0..a[i].max(0) {
                            map.push(i);
                            idx.push(k);
                        }
                    }
                }
                let mut nc = c.gather_curves(&map);
                nc.attributes.set(&idx_name, Domain::Curve, AttrData::Int(idx));
                out.curves = Some(nc);
            }
        }
        _ => {
            if let Some(m) = &g.mesh {
                let t = MeshTopo::new(m);
                let ctx = Ctx::Mesh(m, &t);
                let mut cache = Cache::default();
                let s = bools(&ev(&sel, &ctx, d, &mut cache));
                let a = ints(&ev(&amount, &ctx, d, &mut cache));
                let (mut nm, od, idx) = meshops::duplicate(m, d, &s, &a);
                let nt = MeshTopo::new(&nm);
                let mut full = vec![0; nt.size(&nm, od)];
                for (k, v) in idx.into_iter().enumerate() {
                    if k < full.len() {
                        full[k] = v;
                    }
                }
                nm.attributes.set(&idx_name, od, AttrData::Int(full));
                out.mesh = Some(nm);
            }
            if d == Domain::Point {
                if let Some(p) = &g.points {
                    let ctx = Ctx::Points(p);
                    let mut cache = Cache::default();
                    let s = bools(&ev(&sel, &ctx, d, &mut cache));
                    let a = ints(&ev(&amount, &ctx, d, &mut cache));
                    let mut map = Vec::new();
                    let mut idx = Vec::new();
                    for i in 0..p.positions.len() {
                        if s[i] {
                            for k in 0..a[i].max(0) {
                                map.push(i);
                                idx.push(k);
                            }
                        }
                    }
                    let mut np = PointCloud { positions: map.iter().map(|&i| p.positions[i]).collect(), attributes: p.attributes.clone() };
                    np.attributes.gather_domain(Domain::Point, &map);
                    np.attributes.set(&idx_name, Domain::Point, AttrData::Int(idx));
                    out.points = Some(np);
                }
            }
        }
    }
    o.geo("Geometry", out);
    o.set("Duplicate Index", anon_field(&idx_name, AttrType::Int));
}

/// Reorder elements by (group, key), stable, selected elements only.
fn sort_elements(g: &mut Geometry, d: Domain, sel: &Field, group: &Field, key: &Field) {
    let order = |ctx: &Ctx| -> Vec<usize> {
        let mut cache = Cache::default();
        let s = bools(&ev(sel, ctx, d, &mut cache));
        let gi = ints(&ev(group, ctx, d, &mut cache));
        let k = floats(&ev(key, ctx, d, &mut cache));
        let n = s.len();
        let mut picked: Vec<usize> = (0..n).filter(|&i| s[i]).collect();
        picked.sort_by(|&a, &b| gi[a].cmp(&gi[b]).then(k[a].total_cmp(&k[b])).then(a.cmp(&b)));
        let mut it = picked.into_iter();
        (0..n).map(|i| if s[i] { it.next().unwrap_or(i) } else { i }).collect()
    };
    match d {
        Domain::Point => {
            if let Some(m) = &g.mesh {
                let t = MeshTopo::new(m);
                let ord = order(&Ctx::Mesh(m, &t));
                let mut inv = vec![0u32; ord.len()];
                for (new, &old) in ord.iter().enumerate() {
                    inv[old] = new as u32;
                }
                let mut nm = m.clone();
                nm.positions = ord.iter().map(|&i| m.positions[i]).collect();
                for f in &mut nm.faces {
                    for v in &mut f.verts {
                        *v = inv[*v as usize];
                    }
                }
                for e in &mut nm.loose_edges {
                    *e = [inv[e[0] as usize], inv[e[1] as usize]];
                }
                nm.sharp_edges = crate::mesh::remap_edge_set(&m.sharp_edges, &inv);
                nm.seams = crate::mesh::remap_edge_set(&m.seams, &inv);
                if !m.weights.is_empty() {
                    nm.weights = ord.iter().map(|&i| m.weights.get(i).cloned().unwrap_or_default()).collect();
                }
                nm.attributes.gather_domain(Domain::Point, &ord);
                g.mesh = Some(nm);
            }
            if let Some(p) = &g.points {
                let ord = order(&Ctx::Points(p));
                let mut np = p.clone();
                np.positions = ord.iter().map(|&i| p.positions[i]).collect();
                np.attributes.gather_domain(Domain::Point, &ord);
                g.points = Some(np);
            }
        }
        Domain::Face => {
            if let Some(m) = &g.mesh {
                let t = MeshTopo::new(m);
                let ord = order(&Ctx::Mesh(m, &t));
                let mut nm = m.clone();
                nm.faces = ord.iter().map(|&i| m.faces[i].clone()).collect();
                nm.attributes.gather_domain(Domain::Face, &ord);
                let corners: Vec<usize> = ord.iter().flat_map(|&f| t.corners(f)).collect();
                nm.attributes.gather_domain(Domain::Corner, &corners);
                nm.uv_pins.clear();
                g.mesh = Some(nm);
            }
        }
        Domain::Curve => {
            if let Some(c) = &g.curves {
                let ord = order(&Ctx::Curves(c));
                g.curves = Some(c.gather_curves(&ord));
            }
        }
        Domain::Instance => {
            if let Some(i) = &g.instances {
                let ord = order(&Ctx::Instances(i));
                g.instances = Some(i.gather(&ord));
            }
        }
        _ => {}
    }
}

/// Nearest element lookup for Sample Nearest.
struct Nearest {
    kd: Option<KdTree>,
    surface: Option<(Mesh, Domain)>,
}

impl Nearest {
    fn new(g: &Geometry, d: Domain) -> Nearest {
        let pts: Option<Vec<Vec3>> = match d {
            Domain::Point => g.mesh.as_ref().map(|m| m.positions.clone()).or_else(|| g.points.as_ref().map(|p| p.positions.clone())).or_else(|| g.curves.as_ref().map(|c| c.positions.clone())),
            Domain::Edge => g.mesh.as_ref().map(|m| m.edges().iter().map(|e| (m.positions[e[0] as usize] + m.positions[e[1] as usize]) * 0.5).collect()),
            Domain::Instance => g.instances.as_ref().map(|i| i.transforms.iter().map(|t| t.translation_part()).collect()),
            Domain::Curve => g.curves.as_ref().map(|c| (0..c.curves_num()).map(|ci| {
                let r = c.points(ci);
                let n = r.len().max(1) as f32;
                r.fold(Vec3::ZERO, |a, i| a + c.positions[i]) / n
            }).collect()),
            _ => None,
        };
        match pts {
            Some(p) => Nearest { kd: Some(KdTree::new(&p)), surface: None },
            None => Nearest { kd: None, surface: g.mesh.clone().map(|m| (m, d)) },
        }
    }

    fn find(&self, p: Vec3) -> Option<usize> {
        if let Some(kd) = &self.kd {
            return kd.nearest(p).map(|x| x.0);
        }
        let (m, d) = self.surface.as_ref()?;
        let s = Surface::new(m);
        let h = s.nearest(p)?;
        let t = &s.tris[h.tri];
        Some(match d {
            Domain::Corner => {
                let k = (0..3).max_by(|&a, &b| h.bary[a].total_cmp(&h.bary[b])).unwrap_or(0);
                s.corner(h.tri, k)
            }
            _ => t.face as usize,
        })
    }
}

/// Geometry Proximity's target.
struct Proximity {
    kd: Option<KdTree>,
    pts: Vec<Vec3>,
    edges: Vec<[Vec3; 2]>,
    mesh: Option<Mesh>,
}

impl Proximity {
    fn new(g: &Geometry, target: &str) -> Proximity {
        let g = g.clone();
        let mut p = Proximity { kd: None, pts: Vec::new(), edges: Vec::new(), mesh: None };
        match target {
            "POINTS" => {
                p.pts = g.mesh.iter().flat_map(|m| m.positions.iter()).chain(g.points.iter().flat_map(|x| x.positions.iter())).copied().collect();
                p.kd = Some(KdTree::new(&p.pts));
            }
            "EDGES" => {
                if let Some(m) = &g.mesh {
                    p.edges = m.edges().iter().map(|e| [m.positions[e[0] as usize], m.positions[e[1] as usize]]).collect();
                }
            }
            _ => p.mesh = g.mesh.filter(|m| !m.faces.is_empty()),
        }
        p
    }

    fn query(&self, q: Vec3) -> Option<(Vec3, f32)> {
        if let Some(kd) = &self.kd {
            return kd.nearest(q).map(|(i, d2)| (self.pts[i], d2.sqrt()));
        }
        if !self.edges.is_empty() {
            return self.edges.iter().map(|e| closest_on_segment(q, e[0], e[1])).map(|c| (c, c.distance(q))).min_by(|a, b| a.1.total_cmp(&b.1));
        }
        let m = self.mesh.as_ref()?;
        let s = Surface::new(m);
        s.nearest(q).map(|h| (h.point, h.dist))
    }
}

fn raycast(ins: &Ins, o: &mut Outs) {
    let g = ins.geo("Target Geometry");
    let mesh = Arc::new(g.mesh.clone().unwrap_or_default());
    let src = ins.field("Source Position");
    let dir = ins.field("Ray Direction");
    let len = ins.field("Ray Length");
    let interpolated = ins.node.enum_prop("mapping") != "NEAREST";
    let attr = ins.field("Attribute");
    let def = attr.default_val();
    let attr_vals = Arc::new(with_component(&g, Component::Mesh, |ctx| ev(&attr, ctx, Domain::Corner, &mut Cache::default())).unwrap_or_default());
    type Pick = fn(Option<&super::sample::SurfaceHit>) -> Val;
    let outs: [(&str, Val, Pick); 4] = [
        ("Is Hit", Val::Bool(false), |h| Val::Bool(h.is_some())),
        ("Hit Position", Val::Vector(Vec3::ZERO), |h| Val::Vector(h.map_or(Vec3::ZERO, |h| h.point))),
        ("Hit Normal", Val::Vector(Vec3::ZERO), |h| Val::Vector(h.map_or(Vec3::ZERO, |h| h.normal))),
        ("Hit Distance", Val::Float(0.0), |h| Val::Float(h.map_or(0.0, |h| h.dist))),
    ];
    let cast = move |mesh: &Mesh, ctx: &Ctx, d: Domain, cache: &mut Cache, src: &Field, dir: &Field, len: &Field| -> Vec<Option<super::sample::SurfaceHit>> {
        let s = Surface::new(mesh);
        let p = ev(src, ctx, d, cache);
        let dv = ev(dir, ctx, d, cache);
        let l = ev(len, ctx, d, cache);
        (0..p.len()).map(|i| s.ray(p[i].v(), dv[i].v(), l[i].f())).collect()
    };
    for (name, dflt, f) in outs {
        let (m, s, d2, l) = (mesh.clone(), src.clone(), dir.clone(), len.clone());
        o.field(name, special("Raycast", dflt, move |ctx, d, cache| cast(&m, ctx, d, cache, &s, &d2, &l).iter().map(|h| f(h.as_ref())).collect()));
    }
    let (m, s, d2, l) = (mesh, src, dir, len);
    o.field("Attribute", special("Raycast Attribute", def, move |ctx, d, cache| {
        let surf = Surface::new(&m);
        cast(&m, ctx, d, cache, &s, &d2, &l)
            .iter()
            .map(|h| match h {
                Some(h) if interpolated => surf.interpolate(&attr_vals, Domain::Corner, h),
                Some(h) => surf.nearest_value(&attr_vals, Domain::Corner, h),
                None => def,
            })
            .collect()
    }));
}

fn instance_on_points(ins: &Ins, o: &mut Outs) {
    let g = ins.geo("Points");
    let instance = ins.geo("Instance");
    let pick = ins.b("Pick Instance");
    let mut out = Instances::default();
    // Candidate references.
    let candidates: Vec<(InstanceRef, Mat4)> = match (&instance.instances, pick) {
        (Some(i), true) if !i.is_empty() => (0..i.len()).map(|k| (i.references[i.handles[k] as usize].clone(), i.transforms[k])).collect(),
        _ if instance.is_empty() => Vec::new(),
        _ => vec![(InstanceRef::Geometry(Box::new(instance.clone())), Mat4::IDENTITY)],
    };
    let handles: Vec<(u32, Mat4)> = candidates.into_iter().map(|(r, m)| (out.add_reference(r), m)).collect();
    for c in [Component::Mesh, Component::Points, Component::Curves] {
        if !g.has(c) || handles.is_empty() {
            continue;
        }
        let res = with_component(&g, c, |ctx| {
            let mut cache = Cache::default();
            let d = Domain::Point;
            let s = bools(&ev(&ins.field("Selection"), ctx, d, &mut cache));
            let p = vecs(&ev(&field::position(), ctx, d, &mut cache));
            let r = ev(&ins.field("Rotation"), ctx, d, &mut cache);
            let sc = vecs(&ev(&ins.field("Scale"), ctx, d, &mut cache));
            let idx = ints(&ev(&ins.field("Instance Index"), ctx, d, &mut cache));
            let keep: Vec<usize> = (0..p.len()).filter(|&i| s[i]).collect();
            // Point attributes go to the instances.
            let mut attrs = Vec::new();
            let names: Vec<(String, Domain, AttrType)> = match ctx {
                Ctx::Mesh(m, _) => super::topo::mesh_attr_names(m),
                Ctx::Points(pc) => pc.attributes.list.iter().map(|a| (a.name.clone(), a.domain, a.data.ty())).collect(),
                Ctx::Curves(cv) => cv.attributes.list.iter().filter(|a| a.domain == Domain::Point).map(|a| (a.name.clone(), a.domain, a.data.ty())).collect(),
                _ => Vec::new(),
            };
            for (name, _, ty) in names {
                if matches!(name.as_str(), "position" | "radius" | "UVMap" | "uv_map" | "sharp_face" | "material_index" | "handle_left" | "handle_right" | "handle_type_left" | "handle_type_right" | "nurbs_weight" | "tilt") {
                    continue;
                }
                if let Some(v) = read_attr_on(ctx, &name, d) {
                    let kept: Vec<Val> = keep.iter().map(|&i| v[i]).collect();
                    attrs.push((name, AttrData::from_vals(ty, &kept)));
                }
            }
            let items: Vec<(usize, Mat4)> = keep
                .iter()
                .map(|&i| {
                    let k = if pick { idx[i].rem_euclid(handles.len() as i32) as usize } else { 0 };
                    (k, Mat4::from_trs(p[i], r[i].q(), sc[i]) * handles[k].1)
                })
                .collect();
            (items, attrs)
        });
        let Some((items, attrs)) = res else { continue };
        let mut part = Instances { references: out.references.clone(), ..Default::default() };
        for (k, m) in items {
            part.add(handles[k].0, m);
        }
        for (name, data) in attrs {
            part.attributes.set(&name, Domain::Instance, data);
        }
        let mut merged = std::mem::take(&mut out);
        merged.join(&part);
        out = merged;
    }
    o.geo("Instances", Geometry::from_instances(out));
}
