// SPDX-License-Identifier: GPL-3.0-only
//! Curve nodes: primitives, operations (resample, trim, subdivide, fillet,
//! reverse, set type/handles/radius/tilt), conversions (Curve to Mesh,
//! Curve to Points, Fill Curve) and the curve field inputs.
use super::attributes::{interp, AttrData, AttrType, Domain, Val};
use super::curves::{Curves, BEZIER, CATMULL_ROM, HANDLE_ALIGN, HANDLE_AUTO, HANDLE_FREE, HANDLE_VECTOR, NURBS, POLY};
use super::eval::{anonymous_name, Ins, NodeCx, Outs, SocketValue};
use super::field::{self, special, Cache, Ctx, Field, FieldNode};
use super::nodes_geo::{anon_field, bools, ev, floats, ints, masked, vecs, write_attr};
use super::primitives;
use super::{Component, Geometry, PointCloud};
use crate::math::{Mat3, Quat, Vec3};
use crate::mesh::{Face, Mesh};
use std::sync::Arc;

type Src = Vec<(usize, f32)>;

/// A replacement for one curve: points, their sources and cyclic.
struct NewCurve {
    positions: Vec<Vec3>,
    src: Vec<Src>,
    cyclic: bool,
    ty: i32,
}

/// Rebuild `c`, replacing the curves that have a [`NewCurve`]; point
/// attributes are interpolated from the sources, curve attributes kept.
fn rebuild(c: &Curves, new: Vec<Option<NewCurve>>) -> Curves {
    let mut out = Curves { attributes: c.attributes.clone(), ..Default::default() };
    let mut pmap: Vec<Src> = Vec::new();
    let mut replaced = Vec::new();
    for (ci, n) in new.into_iter().enumerate() {
        match n {
            Some(n) => {
                out.positions.extend_from_slice(&n.positions);
                pmap.extend(n.src);
                replaced.push((ci, n.cyclic, n.ty));
            }
            None => {
                for i in c.points(ci) {
                    out.positions.push(c.positions[i]);
                    pmap.push(vec![(i, 1.0)]);
                }
            }
        }
        out.offsets.push(out.positions.len() as u32);
    }
    out.attributes.interp_domain(Domain::Point, &pmap);
    // Handle types do not interpolate.
    for name in ["handle_type_left", "handle_type_right"] {
        if let Some(a) = out.attributes.get_mut(name) {
            for (i, s) in pmap.iter().enumerate() {
                let v = s.iter().max_by(|a, b| a.1.total_cmp(&b.1)).and_then(|x| c.attributes.get(name).map(|o| o.data.get(x.0))).unwrap_or(Val::Int(HANDLE_AUTO));
                a.data.set(i, v);
            }
        }
    }
    for (ci, cyc, ty) in replaced {
        out.set_cyclic(ci, cyc);
        out.set_curve_type(ci, ty);
    }
    out
}

/// Sample curve `ci` at arc lengths.
fn sample_lengths(c: &Curves, ci: usize, lengths: &[f32]) -> (Vec<Vec3>, Vec<Src>, Vec<Vec3>, Vec<Vec3>) {
    let ev = c.evaluate(ci);
    let mut p = Vec::with_capacity(lengths.len());
    let mut s = Vec::with_capacity(lengths.len());
    let mut t = Vec::new();
    let mut n = Vec::new();
    for &l in lengths {
        let (pp, tt, nn) = ev.sample(l);
        p.push(pp);
        t.push(tt);
        n.push(nn);
        s.push(ev.sample_src(l));
    }
    (p, s, t, n)
}

/// Evenly spaced arc lengths: `count` samples (cyclic curves do not repeat
/// the start).
fn even(total: f32, count: usize, cyclic: bool) -> Vec<f32> {
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![0.0];
    }
    let div = if cyclic { count } else { count - 1 } as f32;
    (0..count).map(|i| total * i as f32 / div).collect()
}

fn curves_of(g: &Geometry) -> Option<&Curves> {
    g.curves.as_ref()
}

/// Evaluate fields on curves.
fn on_curves<R>(c: &Curves, f: impl FnOnce(&Ctx) -> R) -> R {
    f(&Ctx::Curves(c))
}

fn curve_out(o: &mut Outs, c: Curves) {
    o.geo("Curve", Geometry::from_curves(c));
}

pub(crate) fn exec(_cx: &NodeCx, ins: &Ins) -> Option<Vec<SocketValue>> {
    let node = ins.node;
    let mut o = Outs::new(node);
    let mode = node.enum_prop("mode").to_string();
    match node.idname.as_str() {
        // Primitives ------------------------------------------------------
        "GeometryNodeCurvePrimitiveCircle" => {
            let res = ins.i("Resolution").max(3) as usize;
            if mode == "POINTS" {
                match primitives::curve_circle_points(res, ins.v("Point 1"), ins.v("Point 2"), ins.v("Point 3")) {
                    Some((c, center)) => {
                        curve_out(&mut o, c);
                        o.val("Center", Val::Vector(center));
                    }
                    None => curve_out(&mut o, Curves::default()),
                }
            } else {
                curve_out(&mut o, primitives::curve_circle(res, ins.f("Radius")));
            }
        }
        "GeometryNodeCurvePrimitiveLine" => {
            let start = ins.v("Start");
            let end = if mode == "DIRECTION" { start + ins.v("Direction").normalize() * ins.f("Length") } else { ins.v("End") };
            curve_out(&mut o, primitives::curve_poly(&[start, end], false));
        }
        "GeometryNodeCurveArc" => {
            let res = ins.i("Resolution").max(2) as usize;
            if mode == "POINTS" {
                let (c, center, normal, r) = primitives::arc_points(res, ins.v("Start"), ins.v("Middle"), ins.v("End"), ins.f("Offset Angle"), ins.b("Connect Center"), ins.b("Invert Arc"));
                curve_out(&mut o, c);
                o.val("Center", Val::Vector(center)).val("Normal", Val::Vector(normal)).val("Radius", Val::Float(r));
            } else {
                curve_out(&mut o, primitives::arc_radius(res, ins.f("Radius"), ins.f("Start Angle"), ins.f("Sweep Angle"), ins.b("Connect Center"), ins.b("Invert Arc")));
            }
        }
        "GeometryNodeCurveSpiral" => {
            curve_out(&mut o, primitives::spiral(ins.i("Resolution").max(1) as usize, ins.f("Rotations"), ins.f("Start Radius"), ins.f("End Radius"), ins.f("Height"), ins.b("Reverse")));
        }
        "GeometryNodeCurveStar" => {
            let (mut c, outer) = primitives::star(ins.i("Points").max(3) as usize, ins.f("Inner Radius"), ins.f("Outer Radius"), ins.f("Twist"));
            let name = anonymous_name();
            c.attributes.set(&name, Domain::Point, AttrData::Bool(outer));
            curve_out(&mut o, c);
            o.set("Outer Points", anon_field(&name, AttrType::Bool));
        }
        "GeometryNodeCurvePrimitiveQuadrilateral" => {
            let pts = [ins.v("Point 1"), ins.v("Point 2"), ins.v("Point 3"), ins.v("Point 4")];
            curve_out(&mut o, primitives::quadrilateral(&mode, ins.f("Width"), ins.f("Height"), ins.f("Bottom Width"), ins.f("Top Width"), ins.f("Offset"), ins.f("Bottom Height"), ins.f("Top Height"), pts));
        }
        "GeometryNodeCurvePrimitiveBezierSegment" => {
            let (s, e) = (ins.v("Start"), ins.v("End"));
            let (sh, eh) = if mode == "OFFSET" { (s + ins.v("Start Handle"), e + ins.v("End Handle")) } else { (ins.v("Start Handle"), ins.v("End Handle")) };
            curve_out(&mut o, primitives::bezier_segment(ins.i("Resolution").max(1) as usize, s, sh, eh, e));
        }
        "GeometryNodeCurveQuadraticBezier" => {
            curve_out(&mut o, primitives::quadratic_bezier(ins.i("Resolution").max(1) as usize, ins.v("Start"), ins.v("Middle"), ins.v("End")));
        }

        // Operations ------------------------------------------------------
        "GeometryNodeCurveLength" => {
            let g = ins.geo("Curve");
            let l = curves_of(&g).map_or(0.0, |c| (0..c.curves_num()).map(|i| c.length(i)).sum());
            o.val("Length", Val::Float(l));
        }
        "GeometryNodeResampleCurve" => {
            let mut g = ins.geo("Curve");
            if let Some(c) = curves_of(&g) {
                let (s, counts, lens) = on_curves(c, |ctx| {
                    let mut cache = Cache::default();
                    (bools(&ev(&ins.field("Selection"), ctx, Domain::Curve, &mut cache)), ints(&ev(&ins.field("Count"), ctx, Domain::Curve, &mut cache)), floats(&ev(&ins.field("Length"), ctx, Domain::Curve, &mut cache)))
                });
                let new = (0..c.curves_num())
                    .map(|ci| {
                        if !s[ci] {
                            return None;
                        }
                        let cyc = c.cyclic(ci);
                        if mode == "EVALUATED" {
                            let ev = c.evaluate(ci);
                            let src = ev.src.iter().map(|&(a, b, f)| vec![(a, 1.0 - f), (b, f)]).collect();
                            return Some(NewCurve { positions: ev.positions, src, cyclic: cyc, ty: POLY });
                        }
                        let total = c.length(ci);
                        let count = if mode == "LENGTH" { ((total / lens[ci].max(1e-4)) as usize + usize::from(!cyc)).max(1) } else { counts[ci].max(1) as usize };
                        let (p, src, _, _) = sample_lengths(c, ci, &even(total, count, cyc));
                        Some(NewCurve { positions: p, src, cyclic: cyc, ty: POLY })
                    })
                    .collect();
                g.curves = Some(rebuild(c, new));
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeTrimCurve" => {
            let mut g = ins.geo("Curve");
            if let Some(c) = curves_of(&g) {
                let by_len = mode == "LENGTH";
                let (s, a, b) = on_curves(c, |ctx| {
                    let mut cache = Cache::default();
                    let (si, ei) = if by_len { ("Start_001", "End_001") } else { ("Start", "End") };
                    (bools(&ev(&ins.field("Selection"), ctx, Domain::Curve, &mut cache)), floats(&ev(&ins.field(si), ctx, Domain::Curve, &mut cache)), floats(&ev(&ins.field(ei), ctx, Domain::Curve, &mut cache)))
                });
                let new = (0..c.curves_num())
                    .map(|ci| {
                        if !s[ci] {
                            return None;
                        }
                        let ev = c.evaluate(ci);
                        let total = ev.total_length();
                        let (mut l0, mut l1) = if by_len { (a[ci], b[ci]) } else { (a[ci] * total, b[ci] * total) };
                        l0 = l0.clamp(0.0, total);
                        l1 = l1.clamp(0.0, total);
                        if l1 < l0 {
                            std::mem::swap(&mut l0, &mut l1);
                        }
                        let mut ls = vec![l0];
                        ls.extend(ev.lengths.iter().copied().filter(|&x| x > l0 + 1e-6 && x < l1 - 1e-6));
                        ls.push(l1);
                        let (p, src, _, _) = sample_lengths(c, ci, &ls);
                        Some(NewCurve { positions: p, src, cyclic: false, ty: POLY })
                    })
                    .collect();
                g.curves = Some(rebuild(c, new));
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeReverseCurve" => {
            let mut g = ins.geo("Curve");
            if let Some(c) = &g.curves {
                let s = on_curves(c, |ctx| bools(&ev(&ins.field("Selection"), ctx, Domain::Curve, &mut Cache::default())));
                let mut c = c.clone();
                c.reverse(&s);
                g.curves = Some(c);
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeSubdivideCurve" => {
            let mut g = ins.geo("Curve");
            if let Some(c) = curves_of(&g) {
                let cuts = on_curves(c, |ctx| ints(&ev(&ins.field("Cuts"), ctx, Domain::Point, &mut Cache::default())));
                g.curves = Some(subdivide(c, &cuts));
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeFilletCurve" => {
            let mut g = ins.geo("Curve");
            if let Some(c) = curves_of(&g) {
                let (r, n) = on_curves(c, |ctx| {
                    let mut cache = Cache::default();
                    (floats(&ev(&ins.field("Radius"), ctx, Domain::Point, &mut cache)), ints(&ev(&ins.field("Count"), ctx, Domain::Point, &mut cache)))
                });
                let poly = mode == "POLY";
                let limit = ins.b("Limit Radius");
                g.curves = Some(fillet(c, &r, &n, poly, limit));
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeSetCurveRadius" | "GeometryNodeSetCurveTilt" => {
            let mut g = ins.geo("Curve");
            let (name, input, def) = if node.idname == "GeometryNodeSetCurveRadius" { ("radius", "Radius", 1.0) } else { ("tilt", "Tilt", 0.0) };
            if let Some(c) = curves_of(&g) {
                let v = on_curves(c, |ctx| {
                    let mut cache = Cache::default();
                    let s = bools(&ev(&ins.field("Selection"), ctx, Domain::Point, &mut cache));
                    let new = ev(&ins.field(input), ctx, Domain::Point, &mut cache).into_iter().map(|v| Val::Float(v.f())).collect();
                    let old = c.point_vals(name, Val::Float(def));
                    masked(new, Some(old), &s, Val::Float(def))
                });
                write_attr(&mut g, Component::Curves, name, Domain::Point, AttrData::from_vals(AttrType::Float, &v));
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeSetSplineCyclic" | "GeometryNodeSetSplineResolution" => {
            let mut g = ins.geo("Geometry");
            let cyc = node.idname == "GeometryNodeSetSplineCyclic";
            if let Some(c) = &mut g.curves {
                let (s, v) = on_curves(c, |ctx| {
                    let mut cache = Cache::default();
                    (bools(&ev(&ins.field("Selection"), ctx, Domain::Curve, &mut cache)), ev(&ins.field(if cyc { "Cyclic" } else { "Resolution" }), ctx, Domain::Curve, &mut cache))
                });
                for ci in 0..c.curves_num() {
                    if s[ci] {
                        if cyc {
                            c.set_cyclic(ci, v[ci].b());
                        } else {
                            c.set_resolution(ci, v[ci].i().max(1));
                        }
                    }
                }
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeCurveSplineType" => {
            let mut g = ins.geo("Curve");
            let ty = match node.enum_prop("spline_type") {
                "BEZIER" => BEZIER,
                "NURBS" => NURBS,
                "CATMULL_ROM" => CATMULL_ROM,
                _ => POLY,
            };
            if let Some(c) = &mut g.curves {
                let s = on_curves(c, |ctx| bools(&ev(&ins.field("Selection"), ctx, Domain::Curve, &mut Cache::default())));
                for ci in 0..c.curves_num() {
                    if !s[ci] || c.curve_type(ci) == ty {
                        continue;
                    }
                    if ty == BEZIER {
                        // Poly corners stay sharp (vector handles), smooth
                        // types get automatic handles.
                        let ht = if c.curve_type(ci) == POLY { HANDLE_VECTOR } else { HANDLE_AUTO };
                        for name in ["handle_type_left", "handle_type_right"] {
                            let r = c.points(ci);
                            let d = c.ensure(name, Domain::Point, Val::Int(HANDLE_AUTO));
                            for i in r {
                                d.set(i, Val::Int(ht));
                            }
                        }
                    }
                    c.set_curve_type(ci, ty);
                }
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeCurveSetHandles" => {
            let mut g = ins.geo("Curve");
            let ht = match node.enum_prop("handle_type") {
                "FREE" => HANDLE_FREE,
                "VECTOR" => HANDLE_VECTOR,
                "ALIGN" => HANDLE_ALIGN,
                _ => HANDLE_AUTO,
            };
            let sides: &[&str] = match mode.as_str() {
                "LEFT" => &["left"],
                "RIGHT" => &["right"],
                _ => &["left", "right"],
            };
            if let Some(c) = &mut g.curves {
                let s = on_curves(c, |ctx| bools(&ev(&ins.field("Selection"), ctx, Domain::Point, &mut Cache::default())));
                materialize_handles(c);
                for side in sides {
                    let d = c.ensure(&format!("handle_type_{side}"), Domain::Point, Val::Int(HANDLE_AUTO));
                    for (i, &x) in s.iter().enumerate() {
                        if x {
                            d.set(i, Val::Int(ht));
                        }
                    }
                }
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }
        "GeometryNodeSetCurveHandlePositions" => {
            let mut g = ins.geo("Curve");
            let side = if mode == "RIGHT" { "right" } else { "left" };
            if let Some(c) = &mut g.curves {
                materialize_handles(c);
                let pos_field = match &*ins.field("Position") {
                    FieldNode::Input { name, .. } if name == "Handle Position Implicit" => field::named_attribute(&format!("handle_{side}"), AttrType::Float3),
                    _ => ins.field("Position"),
                };
                let (s, p, off) = on_curves(c, |ctx| {
                    let mut cache = Cache::default();
                    (bools(&ev(&ins.field("Selection"), ctx, Domain::Point, &mut cache)), vecs(&ev(&pos_field, ctx, Domain::Point, &mut cache)), vecs(&ev(&ins.field("Offset"), ctx, Domain::Point, &mut cache)))
                });
                let d = c.ensure(&format!("handle_{side}"), Domain::Point, Val::Vector(Vec3::ZERO));
                for i in 0..s.len() {
                    if s[i] {
                        d.set(i, Val::Vector(p[i] + off[i]));
                    }
                }
                let t = c.ensure(&format!("handle_type_{side}"), Domain::Point, Val::Int(HANDLE_FREE));
                for i in 0..s.len() {
                    if s[i] && matches!(t.get(i).i(), HANDLE_AUTO | HANDLE_VECTOR) {
                        t.set(i, Val::Int(HANDLE_FREE));
                    }
                }
            }
            curve_out(&mut o, g.curves.take().unwrap_or_default());
        }

        // Conversions -----------------------------------------------------
        "GeometryNodeCurveToMesh" => {
            let g = ins.geo("Curve");
            let profile = ins.geo("Profile Curve").curves;
            let caps = ins.b("Fill Caps");
            let m = match &g.curves {
                Some(c) => {
                    let scale = on_curves(c, |ctx| floats(&ev(&ins.field("Scale"), ctx, Domain::Point, &mut Cache::default())));
                    curve_to_mesh(c, profile.as_ref(), &scale, caps)
                }
                None => Mesh::default(),
            };
            o.geo("Mesh", Geometry::from_mesh(m));
        }
        "GeometryNodeCurveToPoints" => {
            let g = ins.geo("Curve");
            let mut pc = PointCloud::default();
            let (tn, nn, rn) = (anonymous_name(), anonymous_name(), anonymous_name());
            if let Some(c) = &g.curves {
                let mut src: Vec<Src> = Vec::new();
                let mut curve_of = Vec::new();
                let (mut tans, mut nors) = (Vec::new(), Vec::new());
                let (count, len) = (ins.i("Count").max(1) as usize, ins.f("Length").max(1e-4));
                for ci in 0..c.curves_num() {
                    let cyc = c.cyclic(ci);
                    if mode == "EVALUATED" {
                        let ev = c.evaluate(ci);
                        pc.positions.extend_from_slice(&ev.positions);
                        src.extend(ev.src.iter().map(|&(a, b, f)| vec![(a, 1.0 - f), (b, f)]));
                        tans.extend_from_slice(&ev.tangents);
                        nors.extend_from_slice(&ev.normals);
                        curve_of.extend(std::iter::repeat_n(ci, ev.positions.len()));
                        continue;
                    }
                    let total = c.length(ci);
                    let n = if mode == "LENGTH" { (total / len) as usize + usize::from(!cyc) } else { count };
                    let (p, s, t, nr) = sample_lengths(c, ci, &even(total, n.max(1), cyc));
                    curve_of.extend(std::iter::repeat_n(ci, p.len()));
                    pc.positions.extend(p);
                    src.extend(s);
                    tans.extend(t);
                    nors.extend(nr);
                }
                // Point attributes interpolate; curve attributes copy.
                let mut attrs = c.attributes.clone();
                attrs.list.retain(|a| !a.name.starts_with("handle_") && !matches!(a.name.as_str(), "nurbs_weight" | "curve_type" | "cyclic" | "resolution" | "nurbs_order" | "knots_mode" | "normal_mode"));
                attrs.interp_domain(Domain::Point, &src);
                attrs.gather_domain(Domain::Curve, &curve_of);
                for a in &mut attrs.list {
                    a.domain = Domain::Point;
                }
                if !attrs.contains("radius") {
                    attrs.set("radius", Domain::Point, AttrData::Float(vec![1.0; pc.positions.len()]));
                }
                pc.attributes = attrs;
                let rots: Vec<Quat> = tans.iter().zip(&nors).map(|(&t, &n)| Mat3::from_cols(n, t.cross(n), t).to_quat()).collect();
                pc.attributes.set(&tn, Domain::Point, AttrData::Float3(tans));
                pc.attributes.set(&nn, Domain::Point, AttrData::Float3(nors));
                pc.attributes.set(&rn, Domain::Point, AttrData::Quaternion(rots));
            }
            o.geo("Points", Geometry::from_points(pc));
            o.set("Tangent", anon_field(&tn, AttrType::Float3)).set("Normal", anon_field(&nn, AttrType::Float3)).set("Rotation", anon_field(&rn, AttrType::Quaternion));
        }
        "GeometryNodeFillCurve" => {
            let g = ins.geo("Curve");
            let mut m = Mesh::default();
            if let Some(c) = &g.curves {
                for ci in 0..c.curves_num() {
                    let ev = c.evaluate(ci);
                    if ev.positions.len() < 3 {
                        continue;
                    }
                    let base = m.positions.len() as u32;
                    m.positions.extend(ev.positions.iter().map(|p| Vec3::new(p.x, p.y, 0.0)));
                    let mut verts: Vec<u32> = (0..ev.positions.len() as u32).map(|i| base + i).collect();
                    // Face up (+Z).
                    let area: f32 = (0..verts.len()).map(|i| {
                        let (a, b) = (m.positions[verts[i] as usize], m.positions[verts[(i + 1) % verts.len()] as usize]);
                        a.x * b.y - b.x * a.y
                    }).sum();
                    if area < 0.0 {
                        verts.reverse();
                    }
                    m.faces.push(Face::new(verts));
                }
                if mode != "NGONS" {
                    let tris = m.triangulate();
                    m.faces = tris.into_iter().map(|t| Face::new(t.verts.to_vec())).collect();
                }
            }
            o.geo("Mesh", Geometry::from_mesh(m));
        }

        // Inputs ----------------------------------------------------------
        "GeometryNodeSplineParameter" => {
            o.field("Factor", curve_input("Spline Factor", Val::Float(0.0), |c, d| spline_parameter(c, d, 0)));
            o.field("Length", curve_input("Spline Length Param", Val::Float(0.0), |c, d| spline_parameter(c, d, 1)));
            o.field("Index", curve_input("Spline Point Index", Val::Int(0), |c, d| spline_parameter(c, d, 2)));
        }
        "GeometryNodeSplineLength" => {
            o.field("Length", curve_input("Spline Length", Val::Float(0.0), |c, _| (Domain::Curve, (0..c.curves_num()).map(|i| Val::Float(c.length(i))).collect())));
            o.field("Point Count", curve_input("Spline Point Count", Val::Int(0), |c, _| (Domain::Curve, (0..c.curves_num()).map(|i| Val::Int(c.points(i).len() as i32)).collect())));
        }
        "GeometryNodeInputTangent" => {
            o.field("Tangent", curve_input("Curve Tangent", Val::Vector(Vec3::ZERO), |c, _| {
                let mut out = vec![Val::Vector(Vec3::Z); c.points_num()];
                for ci in 0..c.curves_num() {
                    let ev = c.evaluate(ci);
                    for (k, &(a, _, f)) in ev.src.iter().enumerate() {
                        if f == 0.0 {
                            out[a] = Val::Vector(ev.tangents[k]);
                        }
                    }
                }
                (Domain::Point, out)
            }));
        }
        "GeometryNodeInputCurveTilt" => {
            o.field("Tilt", curve_input("Curve Tilt", Val::Float(0.0), |c, _| (Domain::Point, (0..c.points_num()).map(|i| Val::Float(c.tilt(i))).collect())));
        }
        "GeometryNodeInputCurveHandlePositions" => {
            let rel = ins.field("Relative");
            for (k, out) in ["Left", "Right"].iter().enumerate() {
                let r = rel.clone();
                o.field(out, special("Curve Handle Positions", Val::Vector(Vec3::ZERO), move |ctx, d, cache| {
                    let Ctx::Curves(c) = ctx else { return Vec::new() };
                    let rels = bools(&ev(&r, ctx, Domain::Point, cache));
                    let mut vals = c.positions.iter().map(|&p| Val::Vector(p)).collect::<Vec<_>>();
                    for ci in 0..c.curves_num() {
                        if c.curve_type(ci) != BEZIER {
                            continue;
                        }
                        let (l, rr) = c.handles(ci);
                        for (j, i) in c.points(ci).enumerate() {
                            vals[i] = Val::Vector(if k == 0 { l[j] } else { rr[j] });
                        }
                    }
                    for (i, v) in vals.iter_mut().enumerate() {
                        if rels.get(i).copied().unwrap_or(false) {
                            *v = Val::Vector(v.v() - c.positions[i]);
                        }
                    }
                    ctx.adapt(&vals, Domain::Point, d)
                }));
            }
        }
        "GeometryNodeInputSplineCyclic" => {
            o.field("Cyclic", curve_input("Spline Cyclic", Val::Bool(false), |c, _| (Domain::Curve, (0..c.curves_num()).map(|i| Val::Bool(c.cyclic(i))).collect())));
        }
        "GeometryNodeInputSplineResolution" => {
            o.field("Resolution", curve_input("Spline Resolution", Val::Int(0), |c, _| (Domain::Curve, (0..c.curves_num()).map(|i| Val::Int(c.resolution(i) as i32)).collect())));
        }
        "GeometryNodeCurveEndpointSelection" => {
            let (sf, ef) = (ins.field("Start Size"), ins.field("End Size"));
            o.field("Selection", special("Endpoint Selection", Val::Bool(false), move |ctx, d, cache| {
                let Ctx::Curves(c) = ctx else { return Vec::new() };
                let s = ints(&ev(&sf, ctx, Domain::Curve, cache));
                let e = ints(&ev(&ef, ctx, Domain::Curve, cache));
                let mut out = vec![Val::Bool(false); c.points_num()];
                for ci in 0..c.curves_num() {
                    let r = c.points(ci);
                    let n = r.len() as i32;
                    for (j, i) in r.enumerate() {
                        let j = j as i32;
                        out[i] = Val::Bool(j < s[ci] || j >= n - e[ci]);
                    }
                }
                ctx.adapt(&out, Domain::Point, d)
            }));
        }
        "GeometryNodeCurveOfPoint" => {
            let p = ins.field("Point Index");
            let p2 = p.clone();
            o.field("Curve Index", point_query(p, |_, pc, i| pc[i] as i32));
            o.field("Index in Curve", point_query(p2, |c, pc, i| (i - c.offsets[pc[i]] as usize) as i32));
        }
        "GeometryNodeOffsetPointInCurve" => {
            let (p, off) = (ins.field("Point Index"), ins.field("Offset"));
            for (which, out) in ["Is Valid Offset", "Point Index"].iter().enumerate() {
                let (p, off) = (p.clone(), off.clone());
                let def = if which == 0 { Val::Bool(false) } else { Val::Int(0) };
                o.field(out, special("Offset Point in Curve", def, move |ctx, d, cache| {
                    let Ctx::Curves(c) = ctx else { return Vec::new() };
                    let pc = c.point_curve();
                    let ps = ints(&ev(&p, ctx, d, cache));
                    let os = ints(&ev(&off, ctx, d, cache));
                    ps.iter()
                        .zip(&os)
                        .map(|(&i, &k)| {
                            let (valid, idx) = if i < 0 || i as usize >= pc.len() {
                                (false, 0)
                            } else {
                                let ci = pc[i as usize];
                                let r = c.points(ci);
                                let local = i - r.start as i32 + k;
                                let n = r.len() as i32;
                                if c.cyclic(ci) {
                                    (true, r.start as i32 + local.rem_euclid(n))
                                } else if (0..n).contains(&local) {
                                    (true, r.start as i32 + local)
                                } else {
                                    (false, i)
                                }
                            };
                            if which == 0 {
                                Val::Bool(valid)
                            } else {
                                Val::Int(idx)
                            }
                        })
                        .collect()
                }));
            }
        }
        "GeometryNodePointsOfCurve" => {
            let (cf, wf, sf) = (ins.field("Curve Index"), ins.field("Weights"), ins.field("Sort Index"));
            let cf2 = cf.clone();
            o.field("Point Index", special("Points of Curve", Val::Int(0), move |ctx, d, cache| {
                let Ctx::Curves(c) = ctx else { return Vec::new() };
                let w = floats(&ev(&wf, ctx, Domain::Point, cache));
                let cs = ints(&ev(&cf, ctx, d, cache));
                let ss = ints(&ev(&sf, ctx, d, cache));
                cs.iter().zip(&ss).map(|(&ci, &k)| {
                    if ci < 0 || ci as usize >= c.curves_num() {
                        return Val::Int(0);
                    }
                    let mut l: Vec<usize> = c.points(ci as usize).collect();
                    if l.is_empty() {
                        return Val::Int(0);
                    }
                    l.sort_by(|&a, &b| w[a].total_cmp(&w[b]).then(a.cmp(&b)));
                    Val::Int(l[k.rem_euclid(l.len() as i32) as usize] as i32)
                }).collect()
            }));
            o.field("Total", special("Points of Curve Total", Val::Int(0), move |ctx, d, cache| {
                let Ctx::Curves(c) = ctx else { return Vec::new() };
                ints(&ev(&cf2, ctx, d, cache)).iter().map(|&ci| Val::Int(if ci >= 0 && (ci as usize) < c.curves_num() { c.points(ci as usize).len() as i32 } else { 0 })).collect()
            }));
        }
        "GeometryNodeSampleCurve" => sample_curve(ins, &mut o),
        _ => return None,
    }
    Some(o.done())
}


fn curve_input(name: &str, def: Val, f: fn(&Curves, Domain) -> (Domain, Vec<Val>)) -> Field {
    field::input(name, def, move |ctx, d| match ctx {
        Ctx::Curves(c) => Some(f(c, d)),
        _ => None,
    })
}

fn point_query(p: Field, f: fn(&Curves, &[usize], usize) -> i32) -> Field {
    special("Curve of Point", Val::Int(0), move |ctx, d, cache| {
        let Ctx::Curves(c) = ctx else { return Vec::new() };
        let pc = c.point_curve();
        ev(&p, ctx, d, cache).iter().map(|x| {
            let i = x.i();
            Val::Int(if i >= 0 && (i as usize) < pc.len() { f(c, &pc, i as usize) } else { 0 })
        }).collect()
    })
}

/// Spline Parameter: 0 factor, 1 length, 2 index in curve.
fn spline_parameter(c: &Curves, d: Domain, which: usize) -> (Domain, Vec<Val>) {
    if d == Domain::Curve {
        let lens: Vec<f32> = (0..c.curves_num()).map(|i| c.length(i)).collect();
        let total: f32 = lens.iter().sum();
        let mut acc = 0.0;
        let vals = lens.iter().enumerate().map(|(i, l)| {
            let v = match which {
                0 => if total > 0.0 { acc / total } else { 0.0 },
                1 => acc,
                _ => return Val::Int(i as i32),
            };
            acc += l;
            Val::Float(v)
        }).collect();
        return (Domain::Curve, vals);
    }
    let mut out = vec![if which == 2 { Val::Int(0) } else { Val::Float(0.0) }; c.points_num()];
    for ci in 0..c.curves_num() {
        let r = c.points(ci);
        if which == 2 {
            for (j, i) in r.enumerate() {
                out[i] = Val::Int(j as i32);
            }
            continue;
        }
        let ev = c.evaluate(ci);
        let total = ev.total_length();
        // Length at each control point: where its evaluated point starts.
        let mut at = vec![None; r.len()];
        for (k, &(a, _, f)) in ev.src.iter().enumerate() {
            if f == 0.0 && at[a - r.start].is_none() {
                at[a - r.start] = Some(ev.lengths[k]);
            }
        }
        // Fallback: control polygon lengths.
        let mut poly = 0.0;
        for (j, i) in r.clone().enumerate() {
            if j > 0 {
                poly += c.positions[i].distance(c.positions[i - 1]);
            }
            let l = at[j].unwrap_or(poly);
            out[i] = Val::Float(if which == 0 { if total > 0.0 { l / total } else { 0.0 } } else { l });
        }
    }
    (Domain::Point, out)
}

/// Store the current (possibly automatic) handles as explicit positions so
/// changing handle types keeps the shape.
fn materialize_handles(c: &mut Curves) {
    let n = c.points_num();
    let mut left = c.positions.clone();
    let mut right = c.positions.clone();
    for ci in 0..c.curves_num() {
        if c.curve_type(ci) != BEZIER {
            continue;
        }
        let (l, r) = c.handles(ci);
        for (j, i) in c.points(ci).enumerate() {
            left[i] = l[j];
            right[i] = r[j];
        }
    }
    let types_l: Vec<Val> = c.point_vals("handle_type_left", Val::Int(if c.attributes.contains("handle_left") { HANDLE_FREE } else { HANDLE_AUTO }));
    let types_r: Vec<Val> = c.point_vals("handle_type_right", Val::Int(if c.attributes.contains("handle_right") { HANDLE_FREE } else { HANDLE_AUTO }));
    c.attributes.set("handle_left", Domain::Point, AttrData::Float3(left));
    c.attributes.set("handle_right", Domain::Point, AttrData::Float3(right));
    c.attributes.set("handle_type_left", Domain::Point, AttrData::from_vals(AttrType::Int, &types_l));
    c.attributes.set("handle_type_right", Domain::Point, AttrData::from_vals(AttrType::Int, &types_r));
    let _ = n;
}

fn lerp_src(a: usize, b: usize, t: f32) -> Src {
    vec![(a, 1.0 - t), (b, t)]
}

/// The control points of the part `[t0, t1]` of a cubic Bezier.
fn bezier_part(p: [Vec3; 4], t0: f32, t1: f32) -> [Vec3; 4] {
    let split = |p: [Vec3; 4], t: f32| -> ([Vec3; 4], [Vec3; 4]) {
        let a = p[0].lerp(p[1], t);
        let b = p[1].lerp(p[2], t);
        let c = p[2].lerp(p[3], t);
        let d = a.lerp(b, t);
        let e = b.lerp(c, t);
        let f = d.lerp(e, t);
        ([p[0], a, d, f], [f, e, c, p[3]])
    };
    let (_, right) = split(p, t0);
    let s = if t0 < 1.0 { (t1 - t0) / (1.0 - t0) } else { 0.0 };
    split(right, s).0
}

/// Subdivide Curve: `cuts[i]` points inserted after control point `i`.
fn subdivide(c: &Curves, cuts: &[i32]) -> Curves {
    let mut out = Curves { attributes: c.attributes.clone(), ..Default::default() };
    let mut pmap: Vec<Src> = Vec::new();
    let mut handles: Vec<Option<(Vec3, Vec3)>> = Vec::new();
    for ci in 0..c.curves_num() {
        let r = c.points(ci);
        let n = r.len();
        let cyc = c.cyclic(ci);
        let bez = c.curve_type(ci) == BEZIER;
        let (hl, hr) = if bez { c.handles(ci) } else { (Vec::new(), Vec::new()) };
        let segs = if cyc { n } else { n.saturating_sub(1) };
        let mut pts: Vec<(Vec3, Src, Option<(Vec3, Vec3)>)> = (0..n).map(|j| (c.positions[r.start + j], vec![(r.start + j, 1.0)], bez.then(|| (hl[j], hr[j])))).collect();
        let mut out_pts: Vec<(Vec3, Src, Option<(Vec3, Vec3)>)> = Vec::new();
        for j in 0..n {
            out_pts.push(pts[j].clone());
            if j >= segs {
                continue;
            }
            let k = cuts.get(r.start + j).copied().unwrap_or(0).max(0) as usize;
            if k == 0 {
                continue;
            }
            let jn = (j + 1) % n;
            let (a, b) = (r.start + j, r.start + jn);
            if bez {
                let ctrl = [pts[j].0, hr[j], hl[jn], pts[jn].0];
                let parts: Vec<[Vec3; 4]> = (0..=k).map(|s| bezier_part(ctrl, s as f32 / (k + 1) as f32, (s + 1) as f32 / (k + 1) as f32)).collect();
                // Outgoing handle of this point, incoming of the next.
                if let Some(h) = &mut out_pts.last_mut().unwrap().2 {
                    h.1 = parts[0][1];
                }
                for s in 1..=k {
                    let t = s as f32 / (k + 1) as f32;
                    out_pts.push((parts[s][0], lerp_src(a, b, t), Some((parts[s - 1][2], parts[s][1]))));
                }
                if let Some(h) = &mut pts[jn].2 {
                    h.0 = parts[k][2];
                }
                if jn == 0 {
                    if let Some(h) = &mut out_pts[0].2 {
                        h.0 = parts[k][2];
                    }
                }
            } else {
                for s in 1..=k {
                    let t = s as f32 / (k + 1) as f32;
                    out_pts.push((pts[j].0.lerp(pts[jn].0, t), lerp_src(a, b, t), None));
                }
            }
        }
        for (p, s, h) in out_pts {
            out.positions.push(p);
            pmap.push(s);
            handles.push(h);
        }
        out.offsets.push(out.positions.len() as u32);
    }
    out.attributes.interp_domain(Domain::Point, &pmap);
    if handles.iter().any(|h| h.is_some()) {
        let left: Vec<Vec3> = handles.iter().zip(&out.positions).map(|(h, p)| h.map_or(*p, |h| h.0)).collect();
        let right: Vec<Vec3> = handles.iter().zip(&out.positions).map(|(h, p)| h.map_or(*p, |h| h.1)).collect();
        out.attributes.set("handle_left", Domain::Point, AttrData::Float3(left));
        out.attributes.set("handle_right", Domain::Point, AttrData::Float3(right));
        let n = out.positions.len();
        out.attributes.set("handle_type_left", Domain::Point, AttrData::Int(vec![HANDLE_FREE; n]));
        out.attributes.set("handle_type_right", Domain::Point, AttrData::Int(vec![HANDLE_FREE; n]));
    }
    out
}

/// Fillet Curve: round each corner with an arc of radius `r` (`counts`
/// points per arc in poly mode, 8 otherwise); the result is poly.
fn fillet(c: &Curves, radii: &[f32], counts: &[i32], poly: bool, limit: bool) -> Curves {
    let new = (0..c.curves_num())
        .map(|ci| {
            let r = c.points(ci);
            let n = r.len();
            let cyc = c.cyclic(ci);
            if n < 3 {
                return None;
            }
            let p = &c.positions[r.clone()];
            let mut pos = Vec::new();
            let mut src = Vec::new();
            for j in 0..n {
                let i = r.start + j;
                let interior = cyc || (j > 0 && j + 1 < n);
                let rad = radii.get(i).copied().unwrap_or(0.0);
                if !interior || rad <= 0.0 {
                    pos.push(p[j]);
                    src.push(vec![(i, 1.0)]);
                    continue;
                }
                let (a, b) = (p[(j + n - 1) % n], p[(j + 1) % n]);
                let (d1, d2) = ((a - p[j]).normalize(), (b - p[j]).normalize());
                let theta = d1.dot(d2).clamp(-1.0, 1.0).acos();
                if theta < 1e-4 || (std::f32::consts::PI - theta) < 1e-4 {
                    pos.push(p[j]);
                    src.push(vec![(i, 1.0)]);
                    continue;
                }
                let mut t = rad / (theta * 0.5).tan();
                if limit {
                    t = t.min(a.distance(p[j]) * 0.5).min(b.distance(p[j]) * 0.5);
                }
                let rad = t * (theta * 0.5).tan();
                let center = p[j] + (d1 + d2).normalize() * (rad / (theta * 0.5).sin());
                let (s, e) = (p[j] + d1 * t, p[j] + d2 * t);
                let k = if poly { counts.get(i).copied().unwrap_or(1).max(1) as usize } else { 8 };
                let (u, v) = ((s - center).normalize(), (e - center).normalize());
                let ang = u.dot(v).clamp(-1.0, 1.0).acos();
                let axis = u.cross(v).normalize();
                for q in 0..=k {
                    let f = q as f32 / k as f32;
                    pos.push(center + Quat::from_axis_angle(axis, ang * f).rotate(u) * rad);
                    src.push(vec![(i, 1.0)]);
                }
            }
            Some(NewCurve { positions: pos, src, cyclic: cyc, ty: POLY })
        })
        .collect();
    rebuild(c, new)
}

/// Sweep `profile` along each curve (Blender's Curve to Mesh).
fn curve_to_mesh(c: &Curves, profile: Option<&Curves>, scale: &[f32], caps: bool) -> Mesh {
    let mut m = Mesh::default();
    let profiles: Vec<(Vec<Vec3>, bool)> = match profile {
        Some(p) if p.points_num() > 0 => (0..p.curves_num()).map(|i| {
            let e = p.evaluate(i);
            (e.positions, e.cyclic)
        }).collect(),
        _ => vec![(vec![Vec3::ZERO], false)],
    };
    for ci in 0..c.curves_num() {
        let ev = c.evaluate(ci);
        let rad: Vec<f32> = {
            let r = c.point_vals("radius", Val::Float(1.0));
            let s: Vec<Val> = (0..c.points_num()).map(|i| Val::Float(r[i].f() * scale.get(i).copied().unwrap_or(1.0))).collect();
            c.interp_evaluated(&ev, &s).iter().map(|v| v.f()).collect()
        };
        let ne = ev.positions.len();
        if ne == 0 {
            continue;
        }
        let main_segs = if ev.cyclic { ne } else { ne - 1 };
        for (prof, pcyc) in &profiles {
            let np = prof.len();
            let base = m.positions.len() as u32;
            for i in 0..ne {
                let (t, n) = (ev.tangents[i], ev.normals[i]);
                let b = t.cross(n);
                for q in prof {
                    m.positions.push(ev.positions[i] + (n * q.x + b * q.y + t * q.z) * rad[i]);
                }
            }
            let id = |i: usize, j: usize| base + ((i % ne) * np + (j % np)) as u32;
            if np == 1 {
                for i in 0..main_segs {
                    m.loose_edges.push([id(i, 0), id(i + 1, 0)]);
                }
                continue;
            }
            let prof_segs = if *pcyc { np } else { np - 1 };
            if ne == 1 {
                for j in 0..prof_segs {
                    m.loose_edges.push([id(0, j), id(0, j + 1)]);
                }
                continue;
            }
            for i in 0..main_segs {
                for j in 0..prof_segs {
                    m.faces.push(Face::new(vec![id(i, j), id(i, j + 1), id(i + 1, j + 1), id(i + 1, j)]));
                }
            }
            if caps && *pcyc && !ev.cyclic && np > 2 {
                m.faces.push(Face::new((0..np).map(|j| id(0, j)).collect()));
                m.faces.push(Face::new((0..np).rev().map(|j| id(ne - 1, j)).collect()));
            }
        }
    }
    m
}

fn sample_curve(ins: &Ins, o: &mut Outs) {
    let g = ins.geo("Curves");
    let c = Arc::new(g.curves.clone().unwrap_or_default());
    let value = ins.field("Value");
    let def = value.default_val();
    let vals = Arc::new(on_curves(&c, |ctx| ev(&value, ctx, Domain::Point, &mut Cache::default())));
    let by_len = ins.node.enum_prop("mode") == "LENGTH";
    let all = ins.node.bool_prop("use_all_curves");
    let (fac, len, idx) = (ins.field("Factor"), ins.field("Length"), ins.field("Curve Index"));
    let outs: [(&str, Val, usize); 4] = [("Value", def, 0), ("Position", Val::Vector(Vec3::ZERO), 1), ("Tangent", Val::Vector(Vec3::ZERO), 2), ("Normal", Val::Vector(Vec3::ZERO), 3)];
    for (name, dflt, which) in outs {
        let (c, vals, fac, len, idx) = (c.clone(), vals.clone(), fac.clone(), len.clone(), idx.clone());
        o.field(name, special("Sample Curve", dflt, move |ctx, d, cache| {
            let f = floats(&ev(if by_len { &len } else { &fac }, ctx, d, cache));
            let ci = ints(&ev(&idx, ctx, d, cache));
            let lens: Vec<f32> = (0..c.curves_num()).map(|i| c.length(i)).collect();
            let total: f32 = lens.iter().sum();
            let evs: Vec<_> = (0..c.curves_num()).map(|i| c.evaluate(i)).collect();
            (0..f.len()).map(|k| {
                if evs.is_empty() {
                    return dflt;
                }
                let (curve, l) = if all {
                    let mut l = if by_len { f[k] } else { f[k] * total };
                    let mut cur = 0;
                    while cur + 1 < lens.len() && l > lens[cur] {
                        l -= lens[cur];
                        cur += 1;
                    }
                    (cur, l)
                } else {
                    let cur = ci[k].clamp(0, evs.len() as i32 - 1) as usize;
                    (cur, if by_len { f[k] } else { f[k] * lens[cur] })
                };
                let e = &evs[curve];
                let (p, t, n) = e.sample(l);
                match which {
                    1 => Val::Vector(p),
                    2 => Val::Vector(t),
                    3 => Val::Vector(n),
                    _ => {
                        let w: Vec<(Val, f32)> = e.sample_src(l).into_iter().filter(|x| x.0 < vals.len()).map(|(i, w)| (vals[i], w)).collect();
                        if w.is_empty() { dflt } else { AttrType::of_val(&dflt).convert(interp(&w)) }
                    }
                }
            }).collect()
        }));
    }
}
