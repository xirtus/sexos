// SPDX-License-Identifier: GPL-3.0-only
//! Mesh nodes: primitives, operations, topology inputs, Distribute Points
//! on Faces and the mesh conversions.
use super::attributes::{interp, AttrData, AttrType, Domain, Val};
use super::eval::{anonymous_name, Ins, NodeCx, Outs, SocketValue};
use super::field::{self, special, Cache, Ctx, Field};
use super::meshops;
use super::nodes_geo::{anon_field, bools, ev, floats, read_attr_on, store_marks, vecs, write_attr};
use super::primitives::{self, Fill};
use super::topo::{adapt_mesh, mesh_attr_names, MeshTopo};
use super::{realize, Component, Curves, Geometry, PointCloud};
use crate::math::{Quat, Vec3};
use crate::mesh::Mesh;
use crate::render::shader::tex::hash_uint3;
use std::collections::BinaryHeap;

/// Evaluate fields on a mesh.
fn on_mesh<R>(m: &Mesh, f: impl FnOnce(&Ctx, &MeshTopo) -> R) -> R {
    let t = MeshTopo::new(m);
    f(&Ctx::Mesh(m, &t), &t)
}

pub(crate) fn exec(cx: &NodeCx, ins: &Ins) -> Option<Vec<SocketValue>> {
    let node = ins.node;
    let mut o = Outs::new(node);
    match node.idname.as_str() {
        // Primitives ------------------------------------------------------
        "GeometryNodeMeshCube" => {
            let v = |k: &str| ins.i(k).max(1) as usize;
            let m = primitives::cube(ins.v("Size"), [v("Vertices X"), v("Vertices Y"), v("Vertices Z")]);
            uv_out(&mut o, m);
        }
        "GeometryNodeMeshGrid" => {
            let m = primitives::grid(ins.f("Size X"), ins.f("Size Y"), ins.i("Vertices X").max(1) as usize, ins.i("Vertices Y").max(1) as usize);
            uv_out(&mut o, m);
        }
        "GeometryNodeMeshCircle" => {
            let m = primitives::circle(ins.i("Vertices").max(3) as usize, ins.f("Radius"), Fill::from_name(node.enum_prop("fill_type")));
            o.geo("Mesh", Geometry::from_mesh(m));
        }
        "GeometryNodeMeshCylinder" | "GeometryNodeMeshCone" => {
            let cyl = node.idname == "GeometryNodeMeshCylinder";
            let (rt, rb) = if cyl { (ins.f("Radius"), ins.f("Radius")) } else { (ins.f("Radius Top"), ins.f("Radius Bottom")) };
            let (mut m, parts) = primitives::cone(
                ins.i("Vertices").max(3) as usize,
                ins.i("Side Segments").max(1) as usize,
                ins.i("Fill Segments").max(1) as usize,
                rt.max(0.0),
                rb.max(0.0),
                ins.f("Depth"),
                Fill::from_name(node.enum_prop("fill_type")),
            );
            let top = store_marks(&mut m, Domain::Face, parts.top);
            let bottom = store_marks(&mut m, Domain::Face, parts.bottom);
            let side = store_marks(&mut m, Domain::Face, parts.side);
            o.set("Top", top).set("Bottom", bottom).set("Side", side);
            uv_out(&mut o, m);
        }
        "GeometryNodeMeshUVSphere" => {
            let m = primitives::uv_sphere(ins.i("Segments").max(3) as usize, ins.i("Rings").max(2) as usize, ins.f("Radius"));
            uv_out(&mut o, m);
        }
        "GeometryNodeMeshIcoSphere" => {
            let m = primitives::ico_sphere(ins.f("Radius"), ins.i("Subdivisions").clamp(1, 10) as usize);
            uv_out(&mut o, m);
        }
        "GeometryNodeMeshLine" => {
            let start = ins.v("Start Location");
            let off = ins.v("Offset");
            let m = if node.enum_prop("mode") == "END_POINTS" {
                if node.enum_prop("count_mode") == "RESOLUTION" {
                    let res = ins.f("Resolution").max(1e-4);
                    let len = off.distance(start);
                    let count = (len / res) as usize + 1;
                    primitives::line(start, (off - start).normalize() * res, count.max(1))
                } else {
                    let count = ins.i("Count").max(1) as usize;
                    let step = if count > 1 { (off - start) / (count - 1) as f32 } else { Vec3::ZERO };
                    primitives::line(start, step, count)
                }
            } else {
                primitives::line(start, off, ins.i("Count").max(1) as usize)
            };
            o.geo("Mesh", Geometry::from_mesh(m));
        }

        // Operations ------------------------------------------------------
        "GeometryNodeExtrudeMesh" => {
            let mut g = ins.geo("Mesh");
            let mode = node.enum_prop("mode").to_string();
            let mut top = SocketValue::single(Val::Bool(false));
            let mut side = SocketValue::single(Val::Bool(false));
            if let Some(m) = &g.mesh {
                let d = match mode.as_str() {
                    "VERTICES" => Domain::Point,
                    "EDGES" => Domain::Edge,
                    _ => Domain::Face,
                };
                // Edge extrusion offsets live on vertices.
                let od = if d == Domain::Edge { Domain::Point } else { d };
                let (sel, off) = on_mesh(m, |ctx, _| {
                    let mut cache = Cache::default();
                    let s = bools(&ev(&ins.field("Selection"), ctx, d, &mut cache));
                    let off = vecs(&ev(&ins.field("Offset"), ctx, od, &mut cache));
                    let k = floats(&ev(&ins.field("Offset Scale"), ctx, od, &mut cache));
                    (s, off.iter().zip(&k).map(|(o, k)| *o * *k).collect::<Vec<Vec3>>())
                });
                let ex = match d {
                    Domain::Point => meshops::extrude_vertices(m, &sel, &off),
                    Domain::Edge => meshops::extrude_edges(m, &sel, &off),
                    _ => meshops::extrude_faces(m, &sel, &off, ins.b("Individual")),
                };
                let mut nm = ex.mesh;
                let t = MeshTopo::new(&nm);
                let (tv, sv) = (ex.top.on(&nm, &t), ex.side.on(&nm, &t));
                top = store_marks(&mut nm, ex.top.domain(), tv);
                side = store_marks(&mut nm, ex.side.domain(), sv);
                g.mesh = Some(nm);
            }
            o.geo("Mesh", g).set("Top", top).set("Side", side);
        }
        "GeometryNodeFlipFaces" | "GeometryNodeSplitEdges" | "GeometryNodeTriangulate" => {
            let mut g = ins.geo("Mesh");
            if let Some(m) = &g.mesh {
                let d = if node.idname == "GeometryNodeSplitEdges" { Domain::Edge } else { Domain::Face };
                let s = on_mesh(m, |ctx, _| bools(&ev(&ins.field("Selection"), ctx, d, &mut Cache::default())));
                g.mesh = Some(match node.idname.as_str() {
                    "GeometryNodeFlipFaces" => meshops::flip_faces(m, &s),
                    "GeometryNodeSplitEdges" => meshops::split_edges(m, &s),
                    _ => meshops::triangulate(m, &s, ins.i("Minimum Vertices").max(4) as usize, node.enum_prop("quad_method")),
                });
            }
            o.geo("Mesh", g);
        }
        "GeometryNodeDualMesh" => {
            let mut g = ins.geo("Mesh");
            if let Some(m) = &g.mesh {
                g.mesh = Some(meshops::dual_mesh(m));
            }
            o.geo("Dual Mesh", g);
        }
        "GeometryNodeScaleElements" => {
            let mut g = ins.geo("Geometry");
            if let Some(m) = &g.mesh {
                let d = if node.enum_prop("domain") == "EDGE" { Domain::Edge } else { Domain::Face };
                let single = node.enum_prop("scale_mode") == "SINGLE_AXIS";
                let (s, sc, ce, ax) = on_mesh(m, |ctx, _| {
                    let mut cache = Cache::default();
                    (
                        bools(&ev(&ins.field("Selection"), ctx, d, &mut cache)),
                        floats(&ev(&ins.field("Scale"), ctx, d, &mut cache)),
                        vecs(&ev(&ins.field("Center"), ctx, d, &mut cache)),
                        vecs(&ev(&ins.field("Axis"), ctx, d, &mut cache)),
                    )
                });
                g.mesh = Some(meshops::scale_elements(m, d, &s, &sc, &ce, single.then_some(ax.as_slice())));
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeSubdivideMesh" | "GeometryNodeSubdivisionSurface" => {
            let mut g = ins.geo("Mesh");
            let level = ins.i("Level").clamp(0, 6) as u32;
            if let Some(m) = &g.mesh {
                g.mesh = Some(if node.idname == "GeometryNodeSubdivideMesh" { crate::modifiers::subdivide_simple(m, level) } else { crate::modifiers::subdivide_catmull_clark(m, level) });
            }
            o.geo("Mesh", g);
        }
        "GeometryNodeMeshBoolean" => {
            use crate::ops::{boolean, BooleanOp};
            let resolve = |r: &super::InstanceRef| cx.opts.resolve(r);
            let mesh_of = |g: Geometry| realize(&g, &resolve).mesh.unwrap_or_default();
            let a = mesh_of(ins.geo("Mesh 1"));
            let others: Vec<Mesh> = ins.list("Mesh 2").into_iter().map(|v| mesh_of(v.geometry())).filter(|m| !m.faces.is_empty()).collect();
            let op = node.enum_prop("operation");
            let out = match op {
                "UNION" | "INTERSECT" => {
                    let bop = if op == "UNION" { BooleanOp::Union } else { BooleanOp::Intersect };
                    let mut it = others.into_iter();
                    match it.next() {
                        Some(first) => it.fold(first, |acc, m| boolean(&acc, &m, bop)),
                        None => Mesh::default(),
                    }
                }
                _ => others.iter().fold(a, |acc, m| boolean(&acc, m, BooleanOp::Difference)),
            };
            o.geo("Mesh", Geometry::from_mesh(out));
            o.val("Intersecting Edges", Val::Bool(false));
        }
        "GeometryNodeSetShadeSmooth" => {
            let mut g = ins.geo("Geometry");
            if let Some(m) = &g.mesh {
                let edge = node.enum_prop("domain") == "EDGE";
                let (d, name) = if edge { (Domain::Edge, "sharp_edge") } else { (Domain::Face, "sharp_face") };
                let v = on_mesh(m, |ctx, _| {
                    let mut cache = Cache::default();
                    let s = bools(&ev(&ins.field("Selection"), ctx, d, &mut cache));
                    let smooth = ev(&ins.field("Shade Smooth"), ctx, d, &mut cache);
                    let old = read_attr_on(ctx, name, d);
                    super::nodes_geo::masked(smooth.into_iter().map(|x| Val::Bool(!x.b())).collect(), old, &s, Val::Bool(false))
                });
                write_attr(&mut g, Component::Mesh, name, d, AttrData::from_vals(AttrType::Bool, &v));
            }
            o.geo("Geometry", g);
        }
        "GeometryNodeMeshToCurve" => {
            let g = ins.geo("Mesh");
            let mut c = Curves::default();
            if let Some(m) = &g.mesh {
                let s = on_mesh(m, |ctx, _| bools(&ev(&ins.field("Selection"), ctx, Domain::Edge, &mut Cache::default())));
                let chains = meshops::edge_chains(m, &s);
                let mut src = Vec::new();
                for (chain, cyclic) in &chains {
                    let pts: Vec<Vec3> = chain.iter().map(|&v| m.positions[v as usize]).collect();
                    c.add_curve(&pts, super::curves::POLY, *cyclic);
                    src.extend(chain.iter().map(|&v| v as usize));
                }
                let t = MeshTopo::new(m);
                let ctx = Ctx::Mesh(m, &t);
                for (name, _, ty) in mesh_attr_names(m) {
                    if matches!(name.as_str(), "position" | "sharp_face" | "material_index" | "sharp_edge" | "UVMap" | "uv_map") {
                        continue;
                    }
                    if let Some(v) = read_attr_on(&ctx, &name, Domain::Point) {
                        let vals: Vec<Val> = src.iter().map(|&i| v[i]).collect();
                        c.attributes.set(&name, Domain::Point, AttrData::from_vals(ty, &vals));
                    }
                }
            }
            o.geo("Curve", Geometry::from_curves(c));
        }
        "GeometryNodeMeshToPoints" => {
            let g = ins.geo("Mesh");
            let mut pc = PointCloud::default();
            if let Some(m) = &g.mesh {
                let d = match node.enum_prop("mode") {
                    "EDGES" => Domain::Edge,
                    "FACES" => Domain::Face,
                    "CORNERS" => Domain::Corner,
                    _ => Domain::Point,
                };
                on_mesh(m, |ctx, _| {
                    let mut cache = Cache::default();
                    let s = bools(&ev(&ins.field("Selection"), ctx, d, &mut cache));
                    let p = vecs(&ev(&ins.field("Position"), ctx, d, &mut cache));
                    let r = ev(&ins.field("Radius"), ctx, d, &mut cache);
                    let keep: Vec<usize> = (0..s.len()).filter(|&i| s[i]).collect();
                    pc.positions = keep.iter().map(|&i| p[i]).collect();
                    for (name, _, ty) in mesh_attr_names(m) {
                        if matches!(name.as_str(), "position" | "sharp_face" | "material_index" | "sharp_edge") {
                            continue;
                        }
                        if let Some(v) = read_attr_on(ctx, &name, d) {
                            let vals: Vec<Val> = keep.iter().map(|&i| v[i]).collect();
                            pc.attributes.set(&name, Domain::Point, AttrData::from_vals(ty, &vals));
                        }
                    }
                    let rv: Vec<Val> = keep.iter().map(|&i| r[i]).collect();
                    pc.attributes.set("radius", Domain::Point, AttrData::from_vals(AttrType::Float, &rv));
                });
            }
            o.geo("Points", Geometry::from_points(pc));
        }
        "GeometryNodeDistributePointsOnFaces" => distribute(ins, &mut o),

        // Topology inputs -------------------------------------------------
        "GeometryNodeInputMeshEdgeAngle" => {
            o.field("Unsigned Angle", edge_angle(false));
            o.field("Signed Angle", edge_angle(true));
        }
        "GeometryNodeInputMeshEdgeNeighbors" => {
            o.field("Face Count", mesh_input("Edge Neighbors", Val::Int(0), |_, t| (Domain::Edge, t.edge_faces.iter().map(|f| Val::Int(f.len() as i32)).collect())));
        }
        "GeometryNodeInputMeshEdgeVertices" => {
            o.field("Vertex Index 1", mesh_input("Edge Vertex 1", Val::Int(0), |_, t| (Domain::Edge, t.edges.iter().map(|e| Val::Int(e[0] as i32)).collect())));
            o.field("Vertex Index 2", mesh_input("Edge Vertex 2", Val::Int(0), |_, t| (Domain::Edge, t.edges.iter().map(|e| Val::Int(e[1] as i32)).collect())));
            o.field("Position 1", mesh_input("Edge Position 1", Val::Vector(Vec3::ZERO), |m, t| (Domain::Edge, t.edges.iter().map(|e| Val::Vector(m.positions[e[0] as usize])).collect())));
            o.field("Position 2", mesh_input("Edge Position 2", Val::Vector(Vec3::ZERO), |m, t| (Domain::Edge, t.edges.iter().map(|e| Val::Vector(m.positions[e[1] as usize])).collect())));
        }
        "GeometryNodeInputMeshFaceArea" => {
            o.field("Area", mesh_input("Face Area", Val::Float(0.0), |m, _| (Domain::Face, (0..m.faces.len()).map(|f| Val::Float(m.face_area(f))).collect())));
        }
        "GeometryNodeInputMeshFaceNeighbors" => {
            o.field("Vertex Count", mesh_input("Face Vertex Count", Val::Int(0), |m, _| (Domain::Face, m.faces.iter().map(|f| Val::Int(f.verts.len() as i32)).collect())));
            o.field("Face Count", mesh_input("Face Neighbor Count", Val::Int(0), |m, t| {
                (Domain::Face, (0..m.faces.len()).map(|f| {
                    let mut n: Vec<u32> = t.corners(f).flat_map(|c| t.edge_faces[t.corner_edge[c] as usize].iter().copied()).filter(|&x| x as usize != f).collect();
                    n.sort_unstable();
                    n.dedup();
                    Val::Int(n.len() as i32)
                }).collect())
            }));
        }
        "GeometryNodeInputMeshFaceIsPlanar" => {
            let thr = ins.field("Threshold");
            o.field("Planar", special("Is Face Planar", Val::Bool(false), move |ctx, d, cache| {
                let Ctx::Mesh(m, _) = ctx else { return Vec::new() };
                let th = floats(&ev(&thr, ctx, Domain::Face, cache));
                let vals: Vec<Val> = (0..m.faces.len()).map(|f| {
                    let n = m.face_normal(f);
                    let c = m.face_center(f);
                    let dev = m.faces[f].verts.iter().map(|&v| (m.positions[v as usize] - c).dot(n)).fold(0.0f32, |a, x| a.max(x.abs()));
                    Val::Bool(dev <= th[f] * 0.5 + 1e-6)
                }).collect();
                ctx.adapt(&vals, Domain::Face, d)
            }));
        }
        "GeometryNodeInputMeshIsland" => {
            o.field("Island Index", mesh_input("Island Index", Val::Int(0), |m, t| (Domain::Point, islands(m, t).0.into_iter().map(Val::Int).collect())));
            o.field("Island Count", mesh_input("Island Count", Val::Int(0), |m, t| {
                let (_, n) = islands(m, t);
                (Domain::Point, vec![Val::Int(n); m.positions.len()])
            }));
        }
        "GeometryNodeInputMeshVertexNeighbors" => {
            o.field("Vertex Count", mesh_input("Vertex Neighbors", Val::Int(0), |_, t| (Domain::Point, t.vert_edges.iter().map(|e| Val::Int(e.len() as i32)).collect())));
            o.field("Face Count", mesh_input("Vertex Faces", Val::Int(0), |_, t| (Domain::Point, t.vert_faces.iter().map(|f| Val::Int(f.len() as i32)).collect())));
        }
        "GeometryNodeInputShortestEdgePaths" => {
            let end = ins.field("End Vertex");
            let cost = ins.field("Edge Cost");
            for (k, out) in ["Next Vertex Index", "Total Cost"].iter().enumerate() {
                let (e, c) = (end.clone(), cost.clone());
                let def = if k == 0 { Val::Int(0) } else { Val::Float(0.0) };
                o.field(out, special("Shortest Edge Paths", def, move |ctx, d, cache| {
                    let Ctx::Mesh(m, t) = ctx else { return Vec::new() };
                    let ends = bools(&ev(&e, ctx, Domain::Point, cache));
                    let costs = floats(&ev(&c, ctx, Domain::Edge, cache));
                    let (next, total) = shortest_paths(m, t, &ends, &costs);
                    let vals: Vec<Val> = if k == 0 { next.into_iter().map(Val::Int).collect() } else { total.into_iter().map(Val::Float).collect() };
                    ctx.adapt(&vals, Domain::Point, d)
                }));
            }
        }
        "GeometryNodeCornersOfFace" | "GeometryNodeCornersOfVertex" | "GeometryNodeEdgesOfVertex" => {
            let which = node.idname.clone();
            let (idx_name, out_name) = match which.as_str() {
                "GeometryNodeCornersOfFace" => ("Face Index", "Corner Index"),
                "GeometryNodeCornersOfVertex" => ("Vertex Index", "Corner Index"),
                _ => ("Vertex Index", "Edge Index"),
            };
            let index = ins.field(idx_name);
            let weights = ins.field("Weights");
            let sort = ins.field("Sort Index");
            let (w2, i2, wh2) = (weights.clone(), index.clone(), which.clone());
            o.field(out_name, special("Index Query", Val::Int(0), move |ctx, d, cache| {
                let Ctx::Mesh(_, t) = ctx else { return Vec::new() };
                let (lists, wd) = lists_of(t, &which);
                let w = floats(&ev(&weights, ctx, wd, cache));
                let idx = ev(&index, ctx, d, cache);
                let s = ev(&sort, ctx, d, cache);
                idx.iter().zip(s.iter()).map(|(i, k)| {
                    let Some(list) = usize::try_from(i.i()).ok().and_then(|i| lists.get(i)) else { return Val::Int(0) };
                    if list.is_empty() {
                        return Val::Int(0);
                    }
                    let mut l = list.clone();
                    l.sort_by(|&a, &b| w.get(a).copied().unwrap_or(0.0).total_cmp(&w.get(b).copied().unwrap_or(0.0)).then(a.cmp(&b)));
                    Val::Int(l[k.i().rem_euclid(l.len() as i32) as usize] as i32)
                }).collect()
            }));
            o.field("Total", special("Index Query Total", Val::Int(0), move |ctx, d, cache| {
                let Ctx::Mesh(_, t) = ctx else { return Vec::new() };
                let _ = &w2;
                let (lists, _) = lists_of(t, &wh2);
                ev(&i2, ctx, d, cache).iter().map(|i| Val::Int(usize::try_from(i.i()).ok().and_then(|i| lists.get(i)).map_or(0, |l| l.len() as i32))).collect()
            }));
        }
        "GeometryNodeEdgesOfCorner" => {
            let c = ins.field("Corner Index");
            let c2 = c.clone();
            o.field("Next Edge Index", corner_query(c, |t, c| t.corner_edge[c] as i32));
            o.field("Previous Edge Index", corner_query(c2, |t, c| t.corner_edge[t.prev_corner(c)] as i32));
        }
        "GeometryNodeFaceOfCorner" => {
            let c = ins.field("Corner Index");
            let c2 = c.clone();
            o.field("Face Index", corner_query(c, |t, c| t.corner_face[c] as i32));
            o.field("Index in Face", corner_query(c2, |t, c| (c - t.face_offsets[t.corner_face[c] as usize]) as i32));
        }
        "GeometryNodeVertexOfCorner" => {
            o.field("Vertex Index", corner_query(ins.field("Corner Index"), |t, c| t.corner_vert[c] as i32));
        }
        "GeometryNodeOffsetCornerInFace" => {
            let c = ins.field("Corner Index");
            let off = ins.field("Offset");
            o.field("Corner Index", special("Offset Corner in Face", Val::Int(0), move |ctx, d, cache| {
                let Ctx::Mesh(_, t) = ctx else { return Vec::new() };
                let cs = ev(&c, ctx, d, cache);
                let of = ev(&off, ctx, d, cache);
                cs.iter().zip(of.iter()).map(|(c, k)| {
                    let c = c.i();
                    if c < 0 || c as usize >= t.corner_count() {
                        return Val::Int(0);
                    }
                    let f = t.corner_face[c as usize] as usize;
                    let r = t.corners(f);
                    let n = r.len() as i32;
                    Val::Int(r.start as i32 + (c - r.start as i32 + k.i()).rem_euclid(n))
                }).collect()
            }));
        }
        _ => return None,
    }
    Some(o.done())
}


/// Mesh output plus the UV Map output field (the primitive's UVs).
fn uv_out(o: &mut Outs, m: Mesh) {
    o.geo("Mesh", Geometry::from_mesh(m));
    o.field("UV Map", field::named_attribute("UVMap", AttrType::Float3));
}

fn mesh_input(name: &str, def: Val, f: fn(&Mesh, &MeshTopo) -> (Domain, Vec<Val>)) -> Field {
    field::input(name, def, move |ctx, _| match ctx {
        Ctx::Mesh(m, t) => Some(f(m, t)),
        _ => None,
    })
}

fn corner_query(c: Field, f: fn(&MeshTopo, usize) -> i32) -> Field {
    special("Corner Query", Val::Int(0), move |ctx, d, cache| {
        let Ctx::Mesh(_, t) = ctx else { return Vec::new() };
        ev(&c, ctx, d, cache).iter().map(|x| {
            let i = x.i();
            Val::Int(if i >= 0 && (i as usize) < t.corner_count() { f(t, i as usize) } else { 0 })
        }).collect()
    })
}

fn lists_of(t: &MeshTopo, which: &str) -> (Vec<Vec<usize>>, Domain) {
    match which {
        "GeometryNodeCornersOfFace" => ((0..t.face_offsets.len().saturating_sub(1)).map(|f| t.corners(f).collect()).collect(), Domain::Corner),
        "GeometryNodeCornersOfVertex" => (t.vert_corners.iter().map(|v| v.iter().map(|&c| c as usize).collect()).collect(), Domain::Corner),
        _ => (t.vert_edges.iter().map(|v| v.iter().map(|&c| c as usize).collect()).collect(), Domain::Edge),
    }
}

fn edge_angle(signed: bool) -> Field {
    field::input(if signed { "Signed Angle" } else { "Unsigned Angle" }, Val::Float(0.0), move |ctx, _| {
        let Ctx::Mesh(m, t) = ctx else { return None };
        Some((Domain::Edge, t.edge_faces.iter().enumerate().map(|(e, fs)| {
            if fs.len() != 2 {
                return Val::Float(0.0);
            }
            let (a, b) = (fs[0] as usize, fs[1] as usize);
            let (na, nb) = (m.face_normal(a), m.face_normal(b));
            let ang = na.dot(nb).clamp(-1.0, 1.0).acos();
            if !signed {
                return Val::Float(ang);
            }
            let ed = t.edges[e];
            let mid = (m.positions[ed[0] as usize] + m.positions[ed[1] as usize]) * 0.5;
            let convex = (m.face_center(b) - mid).dot(na) < 0.0;
            Val::Float(if convex { ang } else { -ang })
        }).collect()))
    })
}

/// Connected components by edges: per-vertex island index and the count.
fn islands(m: &Mesh, t: &MeshTopo) -> (Vec<i32>, i32) {
    let n = m.positions.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for e in &t.edges {
        let (a, b) = (find(&mut parent, e[0] as usize), find(&mut parent, e[1] as usize));
        if a != b {
            parent[a.max(b)] = a.min(b);
        }
    }
    let mut ids = vec![-1i32; n];
    let mut count = 0;
    let mut out = vec![0; n];
    for v in 0..n {
        let r = find(&mut parent, v);
        if ids[r] < 0 {
            ids[r] = count;
            count += 1;
        }
        out[v] = ids[r];
    }
    (out, count)
}

/// Dijkstra from every end vertex: the next vertex towards the nearest
/// end and the total cost to it.
fn shortest_paths(m: &Mesh, t: &MeshTopo, ends: &[bool], costs: &[f32]) -> (Vec<i32>, Vec<f32>) {
    #[derive(PartialEq)]
    struct St(f32, usize);
    impl Eq for St {}
    impl PartialOrd for St {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for St {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
        }
    }
    let n = m.positions.len();
    let mut dist = vec![f32::INFINITY; n];
    let mut next: Vec<i32> = (0..n as i32).collect();
    let mut heap = BinaryHeap::new();
    for v in 0..n {
        if ends.get(v).copied().unwrap_or(false) {
            dist[v] = 0.0;
            heap.push(St(0.0, v));
        }
    }
    while let Some(St(d, v)) = heap.pop() {
        if d > dist[v] {
            continue;
        }
        for &e in &t.vert_edges[v] {
            let ed = t.edges[e as usize];
            let u = if ed[0] as usize == v { ed[1] as usize } else { ed[0] as usize };
            let nd = d + costs.get(e as usize).copied().unwrap_or(1.0).max(0.0);
            if nd < dist[u] {
                dist[u] = nd;
                next[u] = v as i32;
                heap.push(St(nd, u));
            }
        }
    }
    (next, dist.into_iter().map(|d| if d.is_finite() { d } else { 0.0 }).collect())
}

/// A small deterministic generator (PCG) seeded from hashes.
struct Rng(u64);

impl Rng {
    fn new(seed: u32) -> Rng {
        let mut r = Rng(seed as u64 ^ 0x853c_49e6_748f_ea9b);
        r.next();
        r
    }
    fn next(&mut self) -> u32 {
        let old = self.0;
        self.0 = old.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let x = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        x.rotate_right(rot)
    }
    fn float(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
    fn barycentric(&mut self) -> [f32; 3] {
        let (mut a, mut b) = (self.float(), self.float());
        if a + b > 1.0 {
            a = 1.0 - a;
            b = 1.0 - b;
        }
        [1.0 - a - b, a, b]
    }
}

/// Points scattered on the selected faces, deterministic per seed.
fn distribute(ins: &Ins, o: &mut Outs) {
    let g = ins.geo("Mesh");
    let Some(m) = &g.mesh else {
        o.geo("Points", Geometry::default());
        return;
    };
    let poisson = ins.node.enum_prop("distribute_method") == "POISSON";
    let seed = ins.i("Seed");
    let t = MeshTopo::new(m);
    let ctx = Ctx::Mesh(m, &t);
    let mut cache = Cache::default();
    let sel = bools(&ev(&ins.field("Selection"), &ctx, Domain::Face, &mut cache));
    let density_field = if poisson { ins.field("Density Factor") } else { ins.field("Density") };
    let dens = floats(&ev(&density_field, &ctx, Domain::Corner, &mut cache));
    let max_density = ins.f("Density Max").max(0.0);
    let dist_min = ins.f("Distance Min").max(0.0);
    let tris = m.triangulate();
    let corner = |tr: &crate::mesh::Tri, k: usize| t.face_offsets[tr.face as usize] + tr.corners[k] as usize;
    // (triangle, barycentric, id)
    let mut samples: Vec<(usize, [f32; 3])> = Vec::new();
    for (ti, tr) in tris.iter().enumerate() {
        if !sel.get(tr.face as usize).copied().unwrap_or(false) {
            continue;
        }
        let p = tr.verts.map(|v| m.positions[v as usize]);
        let area = (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
        let density = if poisson { max_density } else { (0..3).map(|k| dens.get(corner(tr, k)).copied().unwrap_or(0.0)).sum::<f32>() / 3.0 };
        let amount = area * density.max(0.0);
        let mut rng = Rng::new(hash_uint3(ti as u32, seed as u32, 0x9e37));
        let add = amount.fract() > rng.float();
        let count = amount as usize + usize::from(add);
        for _ in 0..count {
            samples.push((ti, rng.barycentric()));
        }
    }
    let pos_of = |s: &(usize, [f32; 3])| {
        let tr = &tris[s.0];
        let p = tr.verts.map(|v| m.positions[v as usize]);
        p[0] * s.1[0] + p[1] * s.1[1] + p[2] * s.1[2]
    };
    if poisson {
        // Eliminate points closer than the minimum distance (in order), then
        // keep each with the density factor as probability.
        let mut kept: Vec<(usize, [f32; 3])> = Vec::new();
        let cell = dist_min.max(1e-6);
        let mut grid: std::collections::HashMap<(i64, i64, i64), Vec<Vec3>> = std::collections::HashMap::new();
        let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
        for s in samples {
            let p = pos_of(&s);
            let k = key(p);
            let mut ok = true;
            if dist_min > 0.0 {
                'outer: for dx in -1..=1 {
                    for dy in -1..=1 {
                        for dz in -1..=1 {
                            if let Some(v) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                                if v.iter().any(|q| q.distance(p) < dist_min) {
                                    ok = false;
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
            }
            if ok {
                grid.entry(k).or_default().push(p);
                kept.push(s);
            }
        }
        let mut rng = Rng::new(hash_uint3(seed as u32, 17, 0x5151));
        samples = kept
            .into_iter()
            .filter(|s| {
                let tr = &tris[s.0];
                let f: f32 = (0..3).map(|k| dens.get(corner(tr, k)).copied().unwrap_or(0.0) * s.1[k]).sum();
                rng.float() < f
            })
            .collect();
    }
    let mut pc = PointCloud { positions: samples.iter().map(pos_of).collect(), ..Default::default() };
    // Attributes, interpolated through the corners.
    for (name, _, ty) in mesh_attr_names(m) {
        if matches!(name.as_str(), "position" | "sharp_face" | "material_index" | "sharp_edge" | "id") {
            continue;
        }
        let Some((src, data)) = ctx.read_attr(&name) else { continue };
        let vals = adapt_mesh(m, &t, &data.to_vals(), src, Domain::Corner);
        let out: Vec<Val> = samples.iter().map(|s| {
            let tr = &tris[s.0];
            ty.convert(interp(&(0..3).map(|k| (vals[corner(tr, k)], s.1[k])).collect::<Vec<_>>()))
        }).collect();
        pc.attributes.set(&name, Domain::Point, AttrData::from_vals(ty, &out));
    }
    let ids: Vec<i32> = samples.iter().enumerate().map(|(i, s)| (hash_uint3(s.0 as u32, i as u32, seed as u32) & 0x7fff_ffff) as i32).collect();
    pc.attributes.set("id", Domain::Point, AttrData::Int(ids));
    pc.attributes.set("radius", Domain::Point, AttrData::Float(vec![0.05; samples.len()]));
    let normals: Vec<Vec3> = samples.iter().map(|s| m.face_normal(tris[s.0].face as usize)).collect();
    let (nn, rn) = (anonymous_name(), anonymous_name());
    pc.attributes.set(&rn, Domain::Point, AttrData::Quaternion(normals.iter().map(|&n| Quat::from_rotation_arc(Vec3::Z, n)).collect()));
    pc.attributes.set(&nn, Domain::Point, AttrData::Float3(normals));
    o.geo("Points", Geometry::from_points(pc));
    o.set("Normal", anon_field(&nn, AttrType::Float3));
    o.set("Rotation", anon_field(&rn, AttrType::Quaternion));
}

