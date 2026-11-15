// SPDX-License-Identifier: GPL-3.0-only
//! The rest of Blender's Edit Mode `mesh.*` operators: dissolves,
//! subdivision, rotate/split/rip, fills, connect, spin and screw, symmetry,
//! convex hull, boolean intersect, extrude variants, merge at first/last,
//! Laplacian smooth and the selection operators. The geometry is
//! `sex_3d::ops2`; this module gathers the selection, runs the operator on a
//! copy of the mesh, records one undo step when it succeeds and selects what
//! Blender leaves selected. Defaults are Blender's operator defaults.
use crate::doc::{Doc, EditSelection, Elem, SelectMode};
use crate::edit::{self, active_mesh, selected_edges, selected_faces, selected_verts};
use crate::ui::Op;
use sex_3d::ops::BooleanOp;
use sex_3d::ops2::{self, Selection};
use sex_3d::{Mesh, Vec3};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// What an operator works on, in the active mesh's local space.
struct Ctx {
    verts: Vec<u32>,
    edges: Vec<[u32; 2]>,
    faces: Vec<u32>,
    /// Selected vertices in click order (history first, then the rest).
    order: Vec<u32>,
    /// Vertices in the select history only.
    history: Vec<u32>,
    cursor: Vec3,
    /// World Z (Blender's spin axis seen from the top view) in local space.
    axis: Vec3,
    /// Average normal of the selected faces, if any.
    normal: Option<Vec3>,
}

impl Ctx {
    fn new(d: &Doc, oi: usize, mi: usize) -> Ctx {
        let m = &d.scene.meshes[mi];
        let inv = d.scene.world_matrix(oi).inverse();
        let verts = selected_verts(d);
        let history: Vec<u32> = d.edit.history.iter().filter_map(|e| if let Elem::Vert(v) = e { Some(*v) } else { None }).collect();
        let mut order = history.clone();
        for &v in &verts {
            if !order.contains(&v) {
                order.push(v);
            }
        }
        let faces = selected_faces(d);
        let n = faces.iter().filter(|&&f| (f as usize) < m.faces.len()).fold(Vec3::ZERO, |a, &f| a + m.face_normal(f as usize) * m.face_area(f as usize).max(1e-9)).normalize();
        Ctx {
            verts,
            edges: selected_edges(d),
            faces,
            order,
            history,
            cursor: inv.transform_point(d.cursor),
            axis: inv.transform_vector(Vec3::Z).normalize(),
            normal: (n != Vec3::ZERO).then_some(n),
        }
    }
    fn need_verts(&self, n: usize) -> Result<(), String> {
        if self.verts.len() < n {
            return Err(if n == 1 { "Select vertices".into() } else { format!("Select at least {n} vertices") });
        }
        Ok(())
    }
    fn need_edges(&self) -> Result<(), String> {
        if self.edges.is_empty() {
            return Err("Select edges".into());
        }
        Ok(())
    }
    fn need_faces(&self) -> Result<(), String> {
        if self.faces.is_empty() {
            return Err("Select faces".into());
        }
        Ok(())
    }
}

/// Make `s` the edit selection, read the way the current select mode reads
/// selections (vertex mode chooses vertices, edge mode edges, face mode faces;
/// a result given only as vertices selects what they span).
pub fn set_selection(d: &mut Doc, s: &Selection) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let nv = m.positions.len() as u32;
    let mut e = EditSelection::default();
    e.verts.extend(s.verts.iter().copied().filter(|&v| v < nv));
    for x in &s.edges {
        if x[0] < nv && x[1] < nv {
            e.edges.insert(ekey(x[0], x[1]));
            e.verts.insert(x[0]);
            e.verts.insert(x[1]);
        }
    }
    for &f in &s.faces {
        if let Some(face) = m.faces.get(f as usize) {
            e.faces.insert(f);
            e.verts.extend(face.verts.iter().copied());
            for (a, b) in face.edges() {
                e.edges.insert(ekey(a, b));
            }
        }
    }
    match d.select_mode {
        SelectMode::Edge if s.edges.is_empty() && s.faces.is_empty() => {
            for x in m.edges() {
                if e.verts.contains(&x[0]) && e.verts.contains(&x[1]) {
                    e.edges.insert((x[0], x[1]));
                }
            }
        }
        SelectMode::Face if s.faces.is_empty() => {
            for (fi, f) in m.faces.iter().enumerate() {
                if f.verts.iter().all(|v| e.verts.contains(v)) {
                    e.faces.insert(fi as u32);
                }
            }
        }
        _ => {}
    }
    d.edit = e;
    edit::flush(d);
}

/// Run `f` on a copy of the active mesh; on success record the undo step,
/// keep the result and select what it returns, otherwise report the error.
fn apply(d: &mut Doc, label: &str, f: impl FnOnce(&mut Mesh, &Ctx) -> Result<Selection, String>) -> bool {
    let Some((oi, mi)) = active_mesh(d) else { return false };
    let ctx = Ctx::new(d, oi, mi);
    let mut m = d.scene.meshes[mi].clone();
    match f(&mut m, &ctx) {
        Ok(sel) => {
            d.push_undo(label);
            d.scene.meshes[mi] = m;
            edit::after_topology(d);
            set_selection(d, &sel);
            true
        }
        Err(e) => {
            d.report = e;
            false
        }
    }
}

/// Parse a Blender direction enum ("NEGATIVE_X", "POSITIVE_Z", ...): (axis,
/// negative side to positive side).
fn direction(arg: &str) -> (usize, bool) {
    let axis = match arg.chars().last() {
        Some('Y') => 1,
        Some('Z') => 2,
        _ => 0,
    };
    (axis, !arg.starts_with("POSITIVE"))
}

fn num<T: std::str::FromStr>(arg: &str, default: T) -> T {
    arg.trim().parse().unwrap_or(default)
}

/// Run one of this module's `mesh.*` operators; false for ids it does not know.
pub fn run(d: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    match op.id.as_str() {
        // ---- dissolve / delete
        "mesh.dissolve_verts" => {
            apply(d, "Dissolve Vertices", |m, c| {
                c.need_verts(1)?;
                Ok(ops2::dissolve_verts(m, &c.verts))
            });
        }
        "mesh.dissolve_edges" => {
            apply(d, "Dissolve Edges", |m, c| {
                c.need_edges()?;
                Ok(ops2::dissolve_edges(m, &c.edges, true))
            });
        }
        "mesh.dissolve_faces" => {
            apply(d, "Dissolve Faces", |m, c| {
                c.need_faces()?;
                Ok(ops2::dissolve_faces(m, &c.faces, false))
            });
        }
        "mesh.dissolve_degenerate" => {
            apply(d, "Degenerate Dissolve", |m, c| {
                c.need_verts(1)?;
                Ok(ops2::dissolve_degenerate(m, &c.verts, num(arg, 1e-4)))
            });
        }
        "mesh.delete_edgeloop" => {
            apply(d, "Delete Edge Loop", |m, c| {
                c.need_edges()?;
                Ok(ops2::delete_edgeloop(m, &c.edges))
            });
        }
        "mesh.edge_collapse" => {
            apply(d, "Edge Collapse", |m, c| {
                c.need_edges()?;
                Ok(ops2::edge_collapse(m, &c.edges))
            });
        }
        // ---- subdivide
        "mesh.subdivide" => {
            apply(d, "Subdivide", |m, c| {
                c.need_edges()?;
                Ok(ops2::subdivide(m, &c.edges, num(arg, 1u32).max(1)))
            });
        }
        "mesh.subdivide_edgering" => {
            apply(d, "Subdivide Edge-Ring", |m, c| {
                c.need_edges()?;
                Ok(ops2::subdivide_edgering(m, &c.edges, num(arg, 10u32).max(1), 1.0))
            });
        }
        "mesh.unsubdivide" => {
            apply(d, "Un-Subdivide", |m, c| {
                c.need_verts(1)?;
                Ok(ops2::unsubdivide(m, &c.verts, num(arg, 2u32)))
            });
        }
        "mesh.poke" => {
            apply(d, "Poke Faces", |m, c| {
                c.need_faces()?;
                Ok(ops2::poke(m, &c.faces, 0.0))
            });
        }
        "mesh.tris_convert_to_quads" => {
            apply(d, "Tris to Quads", |m, c| {
                c.need_faces()?;
                let lim = 40f32.to_radians();
                Ok(ops2::tris_to_quads(m, &c.faces, lim, lim))
            });
        }
        "mesh.beautify_fill" => {
            apply(d, "Beautify Faces", |m, c| {
                c.need_faces()?;
                Ok(ops2::beautify_fill(m, &c.faces))
            });
        }
        // ---- edges
        "mesh.edge_rotate" => {
            apply(d, "Rotate Edge", |m, c| {
                c.need_edges()?;
                // In face mode Blender rotates the edge shared by two selected faces.
                let edges: Vec<[u32; 2]> = if c.faces.len() >= 2 {
                    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
                    for &f in &c.faces {
                        for (a, b) in m.faces[f as usize].edges() {
                            *count.entry(ekey(a, b)).or_insert(0) += 1;
                        }
                    }
                    count.into_iter().filter(|&(_, n)| n == 2).map(|(k, _)| [k.0, k.1]).collect()
                } else {
                    c.edges.clone()
                };
                let s = ops2::edge_rotate(m, &edges, arg == "CCW");
                if s.edges.is_empty() {
                    return Err("Could not rotate the selected edges".into());
                }
                Ok(s)
            });
        }
        "mesh.edge_split" => {
            apply(d, "Edge Split", |m, c| {
                c.need_edges()?;
                Ok(ops2::edge_split(m, &c.edges))
            });
        }
        "mesh.split" => {
            apply(d, "Split", |m, c| {
                c.need_faces()?;
                Ok(ops2::split(m, &c.faces))
            });
        }
        "mesh.offset_edge_loops_slide" | "mesh.offset_edge_loops" => {
            apply(d, "Offset Edge Loops", |m, c| {
                c.need_edges()?;
                Ok(ops2::offset_edge_loops(m, &c.edges, 0.25))
            });
        }
        // ---- fill
        "mesh.fill" => {
            apply(d, "Fill", |m, c| {
                c.need_edges()?;
                let s = ops2::fill(m, &c.edges, true);
                if s.faces.is_empty() {
                    return Err("Select closed loops of edges to fill".into());
                }
                Ok(s)
            });
        }
        "mesh.fill_grid" => {
            apply(d, "Grid Fill", |m, c| {
                c.need_edges()?;
                ops2::fill_grid(m, &c.edges)
            });
        }
        "mesh.face_make_planar" => {
            apply(d, "Make Planar Faces", |m, c| {
                c.need_faces()?;
                ops2::face_make_planar(m, &c.faces, 1.0, 1);
                Ok(Selection::faces(m, c.faces.clone()))
            });
        }
        "mesh.face_split_by_edges" => {
            apply(d, "Weld Edges into Faces", |m, c| {
                c.need_edges()?;
                Ok(ops2::face_split_by_edges(m, &c.edges))
            });
        }
        // ---- connect
        "mesh.vert_connect" => {
            apply(d, "Connect Vertex Pairs", |m, c| {
                c.need_verts(2)?;
                let s = ops2::vert_connect(m, &c.verts);
                if s.edges.is_empty() {
                    return Err("No vertex pairs to connect".into());
                }
                let mut s = s;
                s.verts.extend(c.verts.iter().copied());
                Ok(s)
            });
        }
        "mesh.vert_connect_path" => {
            apply(d, "Connect Vertex Path", |m, c| {
                c.need_verts(2)?;
                let s = ops2::vert_connect_path(m, &c.order);
                if s.edges.is_empty() {
                    return Err("Could not connect the vertices".into());
                }
                Ok(s)
            });
        }
        "mesh.vert_connect_concave" => {
            apply(d, "Split Concave Faces", |m, c| {
                c.need_faces()?;
                Ok(ops2::vert_connect_concave(m, &c.faces))
            });
        }
        "mesh.vert_connect_nonplanar" => {
            apply(d, "Split Non-Planar Faces", |m, c| {
                c.need_faces()?;
                Ok(ops2::vert_connect_nonplanar(m, &c.faces, num(arg, 5f32).to_radians()))
            });
        }
        // ---- spin, screw, extrude
        "mesh.spin" => {
            apply(d, "Spin", |m, c| {
                c.need_verts(1)?;
                let angle = num(arg, 90f32).to_radians();
                Ok(ops2::spin(m, &c.faces, &c.edges, &c.verts, c.cursor, c.axis, angle, 12, 0.0))
            });
        }
        "mesh.screw" => {
            apply(d, "Screw", |m, c| {
                c.need_edges()?;
                ops2::screw(m, &c.edges, c.cursor, c.axis, 9, num(arg, 1u32))
            });
        }
        "mesh.extrude_repeat" => {
            apply(d, "Extrude Repeat", |m, c| {
                c.need_verts(1)?;
                let offset = c.normal.unwrap_or(c.axis);
                Ok(ops2::extrude_repeat(m, &c.faces, &c.edges, &c.verts, offset, num(arg, 10u32)))
            });
        }
        // ---- symmetry
        "mesh.symmetrize" => {
            let (axis, neg) = direction(if arg.is_empty() { "NEGATIVE_X" } else { arg });
            apply(d, "Symmetrize", |m, c| {
                c.need_faces()?;
                Ok(ops2::symmetrize(m, &c.faces, axis, neg, 1e-4))
            });
        }
        "mesh.symmetry_snap" => {
            let (axis, neg) = direction(if arg.is_empty() { "NEGATIVE_X" } else { arg });
            apply(d, "Snap to Symmetry", |m, c| {
                c.need_verts(1)?;
                Ok(ops2::symmetry_snap(m, &c.verts, axis, neg, 0.05, 0.5, true))
            });
        }
        // ---- hull, boolean
        "mesh.convex_hull" => {
            apply(d, "Convex Hull", |m, c| {
                c.need_verts(4)?;
                ops2::convex_hull(m, &c.verts, &c.faces)
            });
        }
        "mesh.intersect_boolean" => {
            let op = match arg {
                "UNION" => BooleanOp::Union,
                "INTERSECT" => BooleanOp::Intersect,
                _ => BooleanOp::Difference,
            };
            if apply(d, "Intersect (Boolean)", |m, c| {
                ops2::intersect_boolean(m, &c.faces, op)?;
                Ok(Selection::default())
            }) {
                d.edit_hidden.clear();
            }
        }
        // ---- merge, smooth
        "mesh.merge_first" | "mesh.merge_last" => {
            let first = op.id == "mesh.merge_first";
            apply(d, if first { "Merge at First" } else { "Merge at Last" }, |m, c| {
                c.need_verts(2)?;
                let target = if first { c.history.first() } else { c.history.last() }.copied().ok_or("Click the vertices in order: the merge needs a selection history")?;
                let v = ops2::merge_at_vertex(m, &c.verts, target).ok_or("Nothing to merge")?;
                Ok(Selection::verts(vec![v]))
            });
        }
        "mesh.vertices_smooth_laplacian" => {
            apply(d, "Laplacian Smooth", |m, c| {
                c.need_verts(1)?;
                ops2::smooth_laplacian(m, &c.verts, num(arg, 1u32), 1.0, 5e-5, true);
                Ok(Selection::verts(c.verts.clone()))
            });
        }
        // ---- selection
        "mesh.select_non_manifold" => select_non_manifold(d),
        "mesh.select_nth" => select_nth(d, num(arg, 1usize).max(1)),
        "mesh.select_random" => select_random(d, 0.5, num(arg, 0u64)),
        "mesh.select_mirror" => select_mirror(d, direction(arg).0, false),
        "mesh.shortest_path_select" => shortest_path(d),
        "mesh.select_interior_faces" => {
            let Some((_, mi)) = active_mesh(d) else { return true };
            let faces = ops2::interior_faces(&d.scene.meshes[mi]);
            let s = Selection::faces(&d.scene.meshes[mi], faces);
            set_selection(d, &s);
        }
        "mesh.loop_multi_select_ring" => select_rings(d),
        _ => return false,
    }
    true
}

/// Rip (V) / Rip and Fill: tear at the selection; the torn side becomes the
/// selection, ready for the move that follows.
pub fn rip(d: &mut Doc, fill: bool) -> bool {
    apply(d, if fill { "Rip Fill" } else { "Rip" }, |m, c| {
        c.need_verts(1)?;
        let r = ops2::rip(m, &c.verts, &c.edges, fill);
        if r.selection.verts.is_empty() {
            return Err("Cannot rip here: select vertices or edges with faces on both sides".into());
        }
        Ok(r.selection)
    })
}

/// Rip and Extend (Alt+D): new vertices next to the selected ones, selected
/// for the move that follows.
pub fn rip_edge(d: &mut Doc) -> bool {
    apply(d, "Extend Vertices", |m, c| {
        c.need_verts(1)?;
        Ok(ops2::rip_edge(m, &c.verts))
    })
}

/// Extrude Manifold: returns the world direction to move the region along.
pub fn extrude_manifold(d: &mut Doc) -> Option<Vec3> {
    let n = edit::selection_normal(d);
    apply(d, "Extrude Manifold", |m, c| {
        c.need_faces()?;
        Ok(ops2::extrude_manifold(m, &c.faces))
    })
    .then_some(n)
    .flatten()
}

fn select_non_manifold(d: &mut Doc) {
    if d.select_mode == SelectMode::Face {
        d.report = "Select Non-Manifold does not work in face selection mode".into();
        return;
    }
    let Some((_, mi)) = active_mesh(d) else { return };
    let mut s = ops2::non_manifold(&d.scene.meshes[mi]);
    // Extend the current selection (Blender's default).
    s.verts.extend(d.edit.verts.iter().copied());
    s.edges.extend(d.edit.edges.iter().map(|&(a, b)| [a, b]));
    let hidden = d.edit_hidden.clone();
    s.verts.retain(|v| !hidden.contains(v));
    s.edges.retain(|e| !hidden.contains(&e[0]) && !hidden.contains(&e[1]));
    if d.select_mode == SelectMode::Vertex {
        s.edges.clear();
    }
    set_selection(d, &s);
}

/// Checker Deselect (nth 1, skip 1, offset 0): walk the selection outwards
/// from the active element and deselect every other step.
fn select_nth(d: &mut Doc, nth: usize) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mode = d.select_mode;
    // Elements of the current mode as ids, with adjacency among selected ones.
    let (ids, nbrs): (Vec<u64>, Box<dyn Fn(u64) -> Vec<u64>>) = match mode {
        SelectMode::Vertex => {
            let adj = edit::adjacency(m);
            let sel = d.edit.verts.clone();
            (sel.iter().map(|&v| v as u64).collect(), Box::new(move |v| adj[v as usize].iter().filter(|u| sel.contains(u)).map(|&u| u as u64).collect()))
        }
        SelectMode::Edge => {
            let sel: Vec<(u32, u32)> = d.edit.edges.iter().copied().collect();
            let enc = |e: (u32, u32)| ((e.0 as u64) << 32) | e.1 as u64;
            let mut by_vert: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
            for &e in &sel {
                by_vert.entry(e.0).or_default().push(e);
                by_vert.entry(e.1).or_default().push(e);
            }
            (
                sel.iter().map(|&e| enc(e)).collect(),
                Box::new(move |x| {
                    let e = ((x >> 32) as u32, x as u32);
                    let mut out: Vec<u64> = Vec::new();
                    for v in [e.0, e.1] {
                        out.extend(by_vert[&v].iter().filter(|&&o| o != e).map(|&o| enc(o)));
                    }
                    out
                }),
            )
        }
        SelectMode::Face => {
            let sel = d.edit.faces.clone();
            let mut by_edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
            for &f in &sel {
                for (a, b) in m.faces[f as usize].edges() {
                    by_edge.entry(ekey(a, b)).or_default().push(f);
                }
            }
            let fe: HashMap<u32, Vec<(u32, u32)>> = sel.iter().map(|&f| (f, m.faces[f as usize].edges().map(|(a, b)| ekey(a, b)).collect())).collect();
            (sel.iter().map(|&f| f as u64).collect(), Box::new(move |f| fe[&(f as u32)].iter().flat_map(|k| by_edge[k].iter().copied()).filter(|&g| g as u64 != f).map(|g| g as u64).collect()))
        }
    };
    let start = match (mode, d.edit.active) {
        (SelectMode::Vertex, Some(Elem::Vert(v))) => Some(v as u64),
        (SelectMode::Edge, Some(Elem::Edge(a, b))) => Some(((a as u64) << 32) | b as u64),
        (SelectMode::Face, Some(Elem::Face(f))) => Some(f as u64),
        _ => None,
    }
    .filter(|s| ids.contains(s))
    .or_else(|| ids.first().copied());
    let Some(start) = start else {
        d.report = "Nothing selected".into();
        return;
    };
    let mut depth: HashMap<u64, usize> = HashMap::from([(start, 0)]);
    let mut q = VecDeque::from([start]);
    while let Some(x) = q.pop_front() {
        let dx = depth[&x];
        for y in nbrs(x) {
            if let std::collections::hash_map::Entry::Vacant(e) = depth.entry(y) {
                e.insert(dx + 1);
                q.push_back(y);
            }
        }
    }
    let period = nth + 1;
    let drop: HashSet<u64> = depth.iter().filter(|&(_, &k)| k % period >= nth).map(|(&x, _)| x).collect();
    match mode {
        SelectMode::Vertex => d.edit.verts.retain(|&v| !drop.contains(&(v as u64))),
        SelectMode::Edge => d.edit.edges.retain(|&e| !drop.contains(&(((e.0 as u64) << 32) | e.1 as u64))),
        SelectMode::Face => d.edit.faces.retain(|&f| !drop.contains(&(f as u64))),
    }
    edit::flush(d);
}

/// Select Random (ratio 0.5, action Select): each unhidden element of the
/// select mode is added with probability `ratio`, from a seeded generator.
fn select_random(d: &mut Doc, ratio: f32, seed: u64) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut rnd = move || {
        // xorshift64*
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    };
    let hidden = &d.edit_hidden;
    let mut s = d.edit.clone();
    match d.select_mode {
        SelectMode::Vertex => {
            for v in 0..m.positions.len() as u32 {
                if rnd() < ratio && !hidden.contains(&v) {
                    s.verts.insert(v);
                }
            }
        }
        SelectMode::Edge => {
            for e in m.edges() {
                if rnd() < ratio && !hidden.contains(&e[0]) && !hidden.contains(&e[1]) {
                    s.edges.insert((e[0], e[1]));
                }
            }
        }
        SelectMode::Face => {
            for f in 0..m.faces.len() {
                if rnd() < ratio && !edit::face_hidden(m, f, hidden) {
                    s.faces.insert(f as u32);
                }
            }
        }
    }
    d.edit = s;
    edit::flush(d);
}

/// Select Mirror (axis X, not extending): the elements at the mirrored
/// positions replace the selection.
fn select_mirror(d: &mut Doc, axis: usize, extend: bool) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let eps = 1e-4 * m.bounds().size().length().max(1.0);
    let map = ops2::mirror_vertex_map(m, axis, eps);
    let mv = |v: u32| map.get(v as usize).copied().flatten();
    let mut s = if extend { d.edit.clone() } else { EditSelection::default() };
    match d.select_mode {
        SelectMode::Vertex => s.verts.extend(d.edit.verts.iter().filter_map(|&v| mv(v))),
        SelectMode::Edge => s.edges.extend(d.edit.edges.iter().filter_map(|&(a, b)| Some(ekey(mv(a)?, mv(b)?)))),
        SelectMode::Face => {
            let by_set: HashMap<Vec<u32>, u32> = m
                .faces
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    let mut k = f.verts.clone();
                    k.sort_unstable();
                    (k, i as u32)
                })
                .collect();
            for &f in &d.edit.faces {
                let k: Option<Vec<u32>> = m.faces[f as usize].verts.iter().map(|&v| mv(v)).collect();
                if let Some(mut k) = k {
                    k.sort_unstable();
                    if let Some(&g) = by_set.get(&k) {
                        s.faces.insert(g);
                    }
                }
            }
        }
    }
    s.verts.retain(|v| !d.edit_hidden.contains(v));
    d.edit = s;
    edit::flush(d);
}

/// Select Shortest Path between the last two picked elements (the active
/// one and the one picked before it), added to the selection.
fn shortest_path(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mode = d.select_mode;
    let picks: Vec<Elem> = {
        let mut h: Vec<Elem> = d.edit.history.iter().copied().filter(|e| matches!((mode, e), (SelectMode::Vertex, Elem::Vert(_)) | (SelectMode::Edge, Elem::Edge(..)) | (SelectMode::Face, Elem::Face(_)))).collect();
        if h.len() < 2 {
            // Fall back on exactly two selected elements.
            h = match mode {
                SelectMode::Vertex => d.edit.verts.iter().map(|&v| Elem::Vert(v)).collect(),
                SelectMode::Edge => d.edit.edges.iter().map(|&(a, b)| Elem::Edge(a, b)).collect(),
                SelectMode::Face => d.edit.faces.iter().map(|&f| Elem::Face(f)).collect(),
            };
            if h.len() != 2 {
                d.report = "Path selection requires two matching elements to be selected".into();
                return;
            }
        }
        h[h.len() - 2..].to_vec()
    };
    let mut s = d.edit.clone();
    match (picks[0], picks[1]) {
        (Elem::Vert(a), Elem::Vert(b)) => match ops2::shortest_path_verts(m, a, b) {
            Some(p) => s.verts.extend(p),
            None => d.report = "No path between the vertices".into(),
        },
        (Elem::Edge(a0, a1), Elem::Edge(b0, b1)) => {
            let best = [(a0, b0), (a0, b1), (a1, b0), (a1, b1)]
                .iter()
                .filter_map(|&(x, y)| ops2::shortest_path_verts(m, x, y))
                .min_by(|p, q| {
                    let l = |p: &Vec<u32>| p.windows(2).map(|w| m.positions[w[0] as usize].distance(m.positions[w[1] as usize])).sum::<f32>();
                    l(p).total_cmp(&l(q))
                });
            match best {
                Some(p) => {
                    for w in p.windows(2) {
                        s.edges.insert(ekey(w[0], w[1]));
                    }
                }
                None => d.report = "No path between the edges".into(),
            }
        }
        (Elem::Face(a), Elem::Face(b)) => match ops2::shortest_path_faces(m, a, b) {
            Some(p) => s.faces.extend(p),
            None => d.report = "No path between the faces".into(),
        },
        _ => {}
    }
    d.edit = s;
    edit::flush(d);
}

/// Select Edge Rings through the selected edges.
fn select_rings(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mut edges: BTreeSet<(u32, u32)> = BTreeSet::new();
    for &(a, b) in &d.edit.edges {
        for e in ops2::edge_ring(m, [a, b]) {
            edges.insert(ekey(e[0], e[1]));
        }
    }
    if d.select_mode == SelectMode::Face {
        d.select_mode = SelectMode::Edge;
    }
    let s = Selection::edges(edges.into_iter().map(|(a, b)| [a, b]).collect());
    set_selection(d, &s);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::select_elem;
    use sex_3d::ops;

    fn edit_doc() -> (Doc, usize) {
        let mut d = Doc::default();
        d.run(&Op::with("object.mode_set", "EDIT"));
        let (_, mi) = active_mesh(&d).unwrap();
        (d, mi)
    }

    fn closed(m: &Mesh) -> bool {
        m.validate().is_ok() && m.half_edges().is_closed_manifold()
    }

    #[test]
    fn subdivide_poke_dissolve_undo() {
        let (mut d, mi) = edit_doc();
        d.run(&Op::new("mesh.subdivide"));
        assert_eq!(d.scene.meshes[mi].positions.len(), 26);
        assert_eq!(d.scene.meshes[mi].faces.len(), 24);
        assert_eq!(d.edit.verts.len(), 26);
        assert!(closed(&d.scene.meshes[mi]));
        d.run(&Op::new("mesh.unsubdivide"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 6);
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 24);
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 6);
        // Poke one face; the fan is selected.
        d.run(&Op::with("mesh.select_mode", "FACE"));
        select_elem(&mut d, Some(Elem::Face(0)), false);
        d.run(&Op::new("mesh.poke"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 9);
        assert_eq!(d.edit.faces.len(), 4);
        // Dissolve the fan back into one face.
        d.run(&Op::new("mesh.dissolve_faces"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 6);
        assert_eq!(d.edit.faces.len(), 1);
        assert!(closed(&d.scene.meshes[mi]));
        // Without a selection, operators report and change nothing.
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        let n = d.scene.meshes[mi].faces.len();
        d.run(&Op::new("mesh.poke"));
        assert_eq!(d.scene.meshes[mi].faces.len(), n);
        assert_eq!(d.report, "Select faces");
    }

    #[test]
    fn merge_first_last_use_click_order() {
        let (mut d, mi) = edit_doc();
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(3)), false);
        select_elem(&mut d, Some(Elem::Vert(0)), true);
        let p3 = d.scene.meshes[mi].positions[3];
        d.run(&Op::with("mesh.merge_first", "FIRST"));
        assert_eq!(d.scene.meshes[mi].positions.len(), 7);
        assert_eq!(d.edit.verts.len(), 1);
        let v = *d.edit.verts.iter().next().unwrap();
        assert_eq!(d.scene.meshes[mi].positions[v as usize], p3);
        d.run(&Op::new("ed.undo"));
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(3)), false);
        select_elem(&mut d, Some(Elem::Vert(0)), true);
        let p0 = d.scene.meshes[mi].positions[0];
        d.run(&Op::with("mesh.merge_last", "LAST"));
        let v = *d.edit.verts.iter().next().unwrap();
        assert_eq!(d.scene.meshes[mi].positions[v as usize], p0);
    }

    #[test]
    fn rip_and_extrude_manifold() {
        let (mut d, mi) = edit_doc();
        // Rip a cube edge loop: the top face edges, which tears the top off.
        d.run(&Op::with("mesh.select_mode", "EDGE"));
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Edge(4, 5)), false);
        assert!(rip(&mut d, false));
        assert_eq!(d.scene.meshes[mi].positions.len(), 10);
        assert!(!d.edit.verts.is_empty());
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.meshes[mi].positions.len(), 8);
        // Extrude Manifold of the top face keeps the cube's topology.
        d.run(&Op::with("mesh.select_mode", "FACE"));
        let top = d.scene.meshes[mi].faces.iter().position(|f| f.verts == vec![4, 5, 7, 6]).unwrap() as u32;
        select_elem(&mut d, Some(Elem::Face(top)), false);
        let n = extrude_manifold(&mut d).unwrap();
        assert!(n.z > 0.99);
        assert_eq!(d.scene.meshes[mi].positions.len(), 8);
        assert_eq!(d.edit.faces.len(), 1);
    }

    #[test]
    fn selection_operators() {
        let (mut d, mi) = edit_doc();
        // A closed cube has nothing non-manifold; after deleting a face its rim is.
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        d.run(&Op::new("mesh.select_non_manifold"));
        assert!(d.edit.verts.is_empty());
        ops::delete_faces(&mut d.scene.meshes[mi], &[0]);
        d.run(&Op::new("mesh.select_non_manifold"));
        assert_eq!(d.edit.verts.len(), 4);
        let (mut d, mi) = edit_doc();
        // Mirror vertex 0 (-1,-1,-1) to vertex 1 (+1,-1,-1).
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(0)), false);
        d.run(&Op::new("mesh.select_mirror"));
        assert_eq!(d.edit.verts.iter().copied().collect::<Vec<_>>(), vec![1]);
        // Shortest path across the cube between opposite corners: 3 edges.
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(0)), false);
        select_elem(&mut d, Some(Elem::Vert(7)), true);
        d.run(&Op::new("mesh.shortest_path_select"));
        assert_eq!(d.edit.verts.len(), 4);
        // Checker deselect on a selected grid keeps every other vertex.
        let g = sex_3d::primitives::grid(4, 1, 4.0);
        d.scene.meshes[mi] = g;
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(0)), false);
        d.run(&Op::new("mesh.select_linked"));
        assert_eq!(d.edit.verts.len(), 10);
        d.run(&Op::new("mesh.select_nth"));
        assert_eq!(d.edit.verts.len(), 5);
        // Random selection adds about half; same seed, same result.
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        d.run(&Op::new("mesh.select_random"));
        let a = d.edit.verts.clone();
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        d.run(&Op::new("mesh.select_random"));
        assert_eq!(a, d.edit.verts);
        assert!(!a.is_empty() && a.len() < 10);
        // Edge rings: a rung of the strip selects all five rungs.
        d.run(&Op::with("mesh.select_mode", "EDGE"));
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Edge(1, 6)), false);
        d.run(&Op::new("mesh.loop_multi_select_ring"));
        assert_eq!(d.edit.edges.len(), 5);
    }

    #[test]
    fn hull_symmetrize_and_monkey() {
        let (mut d, mi) = edit_doc();
        d.run(&Op::new("mesh.convex_hull"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 6);
        assert!(closed(&d.scene.meshes[mi]));
        d.run(&Op::with("mesh.select_mode", "FACE"));
        d.run(&Op::with("mesh.select_all", "SELECT"));
        d.run(&Op::new("mesh.symmetrize"));
        assert!(closed(&d.scene.meshes[mi]));
        assert!((d.scene.meshes[mi].volume() - 8.0).abs() < 1e-3);
        let before = d.scene.meshes[mi].positions.len();
        d.run(&Op::new("mesh.primitive_monkey_add"));
        assert!(d.scene.meshes[mi].positions.len() > before);
        // In Object Mode it adds a new object named Monkey.
        d.run(&Op::with("object.mode_set", "OBJECT"));
        let n = d.scene.objects.len();
        d.run(&Op::new("mesh.primitive_monkey_add"));
        assert_eq!(d.scene.objects.len(), n + 1);
        assert!(d.scene.objects[n].name.starts_with("Monkey"));
    }
}
