// SPDX-License-Identifier: GPL-3.0-only
//! Edit Mode on the active mesh: element selection in Blender's three select
//! modes and the `mesh.*` operators (select all/linked/more/less, extrude,
//! inset, bevel, loop cut, knife, fill, merge, delete, normals, duplicate,
//! separate, hide/reveal, ...). Geometry work goes through `sex_3d::ops`;
//! every change is one undo step.
//!
//! Selection follows Blender: in vertex mode the vertices are what is chosen
//! and edges/faces whose vertices are all selected count as selected; in edge
//! mode the edges are chosen; in face mode the faces are. [`flush`] derives
//! the other two sets from the chosen one.
use crate::doc::{Doc, EditSelection, Elem, Mode, SelectMode};
use crate::ui::Op;
use sex_3d::{ops, Mesh, Object, ObjectData, Vec3};
use std::collections::{BTreeSet, HashMap, HashSet};

/// The active object and its mesh index, if it is a mesh.
pub fn active_mesh(d: &Doc) -> Option<(usize, usize)> {
    let a = d.active()?;
    match d.scene.objects[a].data {
        ObjectData::Mesh(mi) if mi < d.scene.meshes.len() => Some((a, mi)),
        _ => None,
    }
}

fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Selected vertices (sorted).
pub fn selected_verts(d: &Doc) -> Vec<u32> {
    d.edit.verts.iter().copied().collect()
}
pub fn selected_faces(d: &Doc) -> Vec<u32> {
    d.edit.faces.iter().copied().collect()
}
pub fn selected_edges(d: &Doc) -> Vec<[u32; 2]> {
    d.edit.edges.iter().map(|&(a, b)| [a, b]).collect()
}

pub fn face_hidden(m: &Mesh, f: usize, hidden: &BTreeSet<u32>) -> bool {
    !hidden.is_empty() && m.faces[f].verts.iter().any(|v| hidden.contains(v))
}

/// Derive the other selection sets from the one the select mode chooses.
pub fn flush_mesh(m: &Mesh, sel: &mut EditSelection, mode: SelectMode, hidden: &BTreeSet<u32>) {
    let nv = m.positions.len() as u32;
    match mode {
        SelectMode::Vertex => {
            sel.verts.retain(|&v| v < nv && !hidden.contains(&v));
            sel.edges = m.edges().into_iter().filter(|e| sel.verts.contains(&e[0]) && sel.verts.contains(&e[1])).map(|e| (e[0], e[1])).collect();
            sel.faces = (0..m.faces.len()).filter(|&f| !m.faces[f].verts.is_empty() && m.faces[f].verts.iter().all(|v| sel.verts.contains(v))).map(|f| f as u32).collect();
        }
        SelectMode::Edge => {
            sel.edges.retain(|&(a, b)| a < nv && b < nv && !hidden.contains(&a) && !hidden.contains(&b));
            sel.verts = sel.edges.iter().flat_map(|&(a, b)| [a, b]).collect();
            sel.faces = (0..m.faces.len()).filter(|&f| !m.faces[f].verts.is_empty() && m.faces[f].edges().all(|(a, b)| sel.edges.contains(&ekey(a, b)))).map(|f| f as u32).collect();
        }
        SelectMode::Face => {
            sel.faces.retain(|&f| (f as usize) < m.faces.len() && !face_hidden(m, f as usize, hidden));
            sel.verts = sel.faces.iter().flat_map(|&f| m.faces[f as usize].verts.iter().copied()).collect();
            sel.edges = sel.faces.iter().flat_map(|&f| m.faces[f as usize].edges().map(|(a, b)| ekey(a, b)).collect::<Vec<_>>()).collect();
        }
    }
    let has = |sel: &EditSelection, e: Elem| match e {
        Elem::Vert(v) => sel.verts.contains(&v),
        Elem::Edge(a, b) => sel.edges.contains(&(a, b)),
        Elem::Face(f) => sel.faces.contains(&f),
    };
    if let Some(a) = sel.active {
        if !has(sel, a) {
            sel.active = None;
        }
    }
    let history = core::mem::take(&mut sel.history);
    sel.history = history.into_iter().filter(|&e| has(sel, e)).collect();
}

pub fn flush(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let mode = d.select_mode;
    let mut sel = core::mem::take(&mut d.edit);
    flush_mesh(&d.scene.meshes[mi], &mut sel, mode, &d.edit_hidden);
    d.edit = sel;
    d.touch();
}

/// Drop selected/hidden indices that no longer exist (after undo).
pub fn sanitize(d: &mut Doc) {
    crate::rig::sanitize(d);
    crate::sculpt::sanitize(d);
    if d.mode != Mode::Edit || crate::rig::edit_armature(d).is_some() || crate::rig::edit_lattice(d).is_some() {
        return;
    }
    let Some((_, mi)) = active_mesh(d) else {
        d.mode = Mode::Object;
        return;
    };
    let nv = d.scene.meshes[mi].positions.len() as u32;
    d.edit_hidden.retain(|&v| v < nv);
    flush(d);
}

/// Average world normal of the selected faces (or of faces around the
/// selected vertices).
pub fn selection_normal(d: &Doc) -> Option<Vec3> {
    let (oi, mi) = active_mesh(d)?;
    let m = &d.scene.meshes[mi];
    let w = d.scene.world_matrix(oi);
    let mut n = Vec3::ZERO;
    for &f in &d.edit.faces {
        if (f as usize) < m.faces.len() {
            n += m.face_normal(f as usize) * m.face_area(f as usize).max(1e-6);
        }
    }
    if n == Vec3::ZERO {
        for (fi, f) in m.faces.iter().enumerate() {
            if f.verts.iter().any(|v| d.edit.verts.contains(v)) {
                n += m.face_normal(fi);
            }
        }
    }
    let n = w.inverse().transpose().transform_vector(n).normalize();
    (n != Vec3::ZERO).then_some(n)
}

/// A / Alt+A / Ctrl+I: SELECT, DESELECT, INVERT or TOGGLE every element.
pub fn select_all(d: &mut Doc, action: &str) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let action = if action == "TOGGLE" {
        if d.edit.verts.is_empty() {
            "SELECT"
        } else {
            "DESELECT"
        }
    } else {
        action
    };
    let hidden = &d.edit_hidden;
    let all_v: BTreeSet<u32> = (0..m.positions.len() as u32).filter(|v| !hidden.contains(v)).collect();
    let all_e: BTreeSet<(u32, u32)> = m.edges().into_iter().map(|e| (e[0], e[1])).filter(|e| !hidden.contains(&e.0) && !hidden.contains(&e.1)).collect();
    let all_f: BTreeSet<u32> = (0..m.faces.len()).filter(|&f| !face_hidden(m, f, hidden)).map(|f| f as u32).collect();
    let s = &mut d.edit;
    match action {
        "DESELECT" => {
            *s = EditSelection::default();
        }
        "INVERT" => match d.select_mode {
            SelectMode::Vertex => s.verts = all_v.difference(&s.verts).copied().collect(),
            SelectMode::Edge => s.edges = all_e.difference(&s.edges).copied().collect(),
            SelectMode::Face => s.faces = all_f.difference(&s.faces).copied().collect(),
        },
        _ => {
            s.verts = all_v;
            s.edges = all_e;
            s.faces = all_f;
        }
    }
    flush(d);
}

/// Select one element (click), extending with `extend` like Blender's
/// Shift+click: unselected -> selected and active; selected but not active
/// -> active; active -> deselected.
pub fn select_elem(d: &mut Doc, e: Option<Elem>, extend: bool) {
    if !extend {
        d.edit = EditSelection::default();
        if let Some(e) = e {
            add_elem(&mut d.edit, e);
            d.edit.active = Some(e);
            d.edit.history.push(e);
        }
        flush(d);
        return;
    }
    let Some(e) = e else { return };
    let selected = match e {
        Elem::Vert(v) => d.edit.verts.contains(&v),
        Elem::Edge(a, b) => d.edit.edges.contains(&(a, b)),
        Elem::Face(f) => d.edit.faces.contains(&f),
    };
    if selected && d.edit.active == Some(e) {
        match e {
            Elem::Vert(v) => {
                d.edit.verts.remove(&v);
            }
            Elem::Edge(a, b) => {
                d.edit.edges.remove(&(a, b));
            }
            Elem::Face(f) => {
                d.edit.faces.remove(&f);
            }
        }
        d.edit.active = None;
        d.edit.history.retain(|&h| h != e);
    } else {
        add_elem(&mut d.edit, e);
        d.edit.active = Some(e);
        d.edit.history.retain(|&h| h != e);
        d.edit.history.push(e);
    }
    flush(d);
}

fn add_elem(s: &mut EditSelection, e: Elem) {
    match e {
        Elem::Vert(v) => {
            s.verts.insert(v);
        }
        Elem::Edge(a, b) => {
            s.edges.insert((a, b));
        }
        Elem::Face(f) => {
            s.faces.insert(f);
        }
    }
}

/// Apply a set of elements to the selection with a box-select mode.
pub fn select_set(d: &mut Doc, hits: Vec<Elem>, mode: crate::doc::BoxMode) {
    use crate::doc::BoxMode as B;
    let mut new = EditSelection::default();
    for e in &hits {
        add_elem(&mut new, *e);
    }
    let s = &mut d.edit;
    fn comb<T: Ord + Copy>(cur: &mut BTreeSet<T>, new: &BTreeSet<T>, mode: B) {
        *cur = match mode {
            B::Set => new.clone(),
            B::Extend => cur.union(new).copied().collect(),
            B::Subtract => cur.difference(new).copied().collect(),
            B::Invert => cur.symmetric_difference(new).copied().collect(),
            B::Intersect => cur.intersection(new).copied().collect(),
        };
    }
    match d.select_mode {
        SelectMode::Vertex => comb(&mut s.verts, &new.verts, mode),
        SelectMode::Edge => comb(&mut s.edges, &new.edges, mode),
        SelectMode::Face => comb(&mut s.faces, &new.faces, mode),
    }
    flush(d);
}

/// Vertex adjacency through edges.
pub(crate) fn adjacency(m: &Mesh) -> Vec<Vec<u32>> {
    let mut adj = vec![Vec::new(); m.positions.len()];
    for e in m.edges() {
        adj[e[0] as usize].push(e[1]);
        adj[e[1] as usize].push(e[0]);
    }
    adj
}

/// Every vertex connected to `seeds`.
fn linked(m: &Mesh, seeds: &[u32], hidden: &BTreeSet<u32>) -> BTreeSet<u32> {
    let adj = adjacency(m);
    let mut out: BTreeSet<u32> = seeds.iter().copied().filter(|v| (*v as usize) < m.positions.len()).collect();
    let mut stack: Vec<u32> = out.iter().copied().collect();
    while let Some(v) = stack.pop() {
        for &n in &adj[v as usize] {
            if !hidden.contains(&n) && out.insert(n) {
                stack.push(n);
            }
        }
    }
    out
}

/// Select the vertices in `verts` in whatever the select mode chooses.
pub(crate) fn select_verts_as_mode(d: &mut Doc, verts: &BTreeSet<u32>, add: bool) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mut s = if add { d.edit.clone() } else { EditSelection::default() };
    s.verts.extend(verts.iter().copied());
    for e in m.edges() {
        if verts.contains(&e[0]) && verts.contains(&e[1]) {
            s.edges.insert((e[0], e[1]));
        }
    }
    for (fi, f) in m.faces.iter().enumerate() {
        if f.verts.iter().all(|v| verts.contains(v)) {
            s.faces.insert(fi as u32);
        }
    }
    s.active = d.edit.active;
    d.edit = s;
    flush(d);
}

/// L under the pointer: add everything connected to `e`.
pub fn select_linked_pick(d: &mut Doc, e: Elem, deselect: bool) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let seeds: Vec<u32> = match e {
        Elem::Vert(v) => vec![v],
        Elem::Edge(a, b) => vec![a, b],
        Elem::Face(f) => m.faces.get(f as usize).map(|f| f.verts.clone()).unwrap_or_default(),
    };
    let set = linked(m, &seeds, &d.edit_hidden);
    if deselect {
        let mut s = d.edit.clone();
        s.verts.retain(|v| !set.contains(v));
        s.edges.retain(|e| !set.contains(&e.0));
        s.faces.retain(|f| !m.faces[*f as usize].verts.iter().any(|v| set.contains(v)));
        d.edit = s;
        flush(d);
    } else {
        select_verts_as_mode(d, &set, true);
    }
}

fn region_faces(d: &Doc) -> Vec<u32> {
    selected_faces(d)
}

/// The mesh of the active object, mutably.
fn mesh_mut(d: &mut Doc) -> Option<&mut Mesh> {
    let (_, mi) = active_mesh(d)?;
    d.scene.meshes.get_mut(mi)
}

pub(crate) fn after_topology(d: &mut Doc) {
    if let Some((_, mi)) = active_mesh(d) {
        let nv = d.scene.meshes[mi].positions.len() as u32;
        d.edit_hidden.retain(|&v| v < nv);
    }
    d.touch();
}

/// Select exactly these faces (face mode semantics in any select mode).
pub fn select_faces(d: &mut Doc, faces: &[u32]) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mut s = EditSelection::default();
    for &f in faces {
        if let Some(face) = m.faces.get(f as usize) {
            s.faces.insert(f);
            s.verts.extend(face.verts.iter().copied());
            for (a, b) in face.edges() {
                s.edges.insert(ekey(a, b));
            }
        }
    }
    d.edit = s;
    let mode = d.select_mode;
    // Keep the chosen set consistent for the current mode.
    let mut sel = core::mem::take(&mut d.edit);
    if mode == SelectMode::Face {
        flush_mesh(m, &mut sel, mode, &d.edit_hidden);
    }
    d.edit = sel;
    d.touch();
}

fn select_vert_list(d: &mut Doc, verts: &[u32]) {
    let set: BTreeSet<u32> = verts.iter().copied().collect();
    select_verts_as_mode(d, &set, false);
    // In vertex/edge modes a lone vertex set chooses verts.
    if d.select_mode != SelectMode::Face {
        let mut s = core::mem::take(&mut d.edit);
        s.verts.extend(set.iter().copied());
        d.edit = s;
        let mode = d.select_mode;
        if mode == SelectMode::Vertex {
            flush(d);
        }
    }
}

/// E: extrude the selection. Returns the world direction to move along
/// (the region normal) when faces were extruded; None means move freely.
pub fn extrude(d: &mut Doc) -> Option<Option<Vec3>> {
    let (oi, mi) = active_mesh(d)?;
    if d.edit.verts.is_empty() {
        d.report = "Nothing selected".into();
        return None;
    }
    d.push_undo("Extrude Region");
    let faces = region_faces(d);
    if !faces.is_empty() {
        let n = selection_normal(d);
        let m = &mut d.scene.meshes[mi];
        ops::extrude_faces(m, &faces, 0.0);
        select_faces(d, &faces);
        after_topology(d);
        let _ = oi;
        return Some(n);
    }
    // Edges become faces, lone vertices become edges.
    let edges = selected_edges(d);
    let verts = selected_verts(d);
    let m = &mut d.scene.meshes[mi];
    let dir: HashMap<(u32, u32), ()> = m.faces.iter().flat_map(|f| f.edges().collect::<Vec<_>>()).map(|e| (e, ())).collect();
    let mut map: HashMap<u32, u32> = HashMap::new();
    for &v in &verts {
        let p = m.positions[v as usize];
        map.insert(v, m.add_vertex(p));
    }
    let mut in_edge: HashSet<u32> = HashSet::new();
    for e in &edges {
        let (a, b) = (e[0], e[1]);
        in_edge.insert(a);
        in_edge.insert(b);
        let (na, nb) = (map[&a], map[&b]);
        // Wind the new quad against the existing face on this edge.
        if dir.contains_key(&(a, b)) {
            m.add_face(&[b, a, na, nb]);
        } else {
            m.add_face(&[a, b, nb, na]);
        }
        m.loose_edges.retain(|le| ekey(le[0], le[1]) != ekey(a, b));
    }
    for &v in &verts {
        if !in_edge.contains(&v) {
            m.loose_edges.push([v, map[&v]]);
        }
    }
    let new: Vec<u32> = verts.iter().map(|v| map[v]).collect();
    let mode = d.select_mode;
    d.select_mode = SelectMode::Vertex;
    select_vert_list(d, &new);
    d.select_mode = mode;
    flush(d);
    after_topology(d);
    Some(None)
}

/// Extrude each selected face on its own.
pub fn extrude_individual(d: &mut Doc) -> bool {
    let Some((_, mi)) = active_mesh(d) else { return false };
    let faces = selected_faces(d);
    if faces.is_empty() {
        d.report = "Select faces to extrude".into();
        return false;
    }
    d.push_undo("Extrude Individual Faces");
    ops::extrude_faces_individual(&mut d.scene.meshes[mi], &faces, 0.0);
    select_faces(d, &faces);
    after_topology(d);
    true
}

/// Shift+D: copy the selected geometry; the copy becomes the selection.
pub fn duplicate(d: &mut Doc) -> bool {
    let Some((_, mi)) = active_mesh(d) else { return false };
    if d.edit.verts.is_empty() {
        return false;
    }
    d.push_undo("Duplicate");
    let faces = selected_faces(d);
    let edges = selected_edges(d);
    let verts = selected_verts(d);
    let m = &mut d.scene.meshes[mi];
    let mut map: HashMap<u32, u32> = HashMap::new();
    for &v in &verts {
        let p = m.positions[v as usize];
        map.insert(v, m.add_vertex(p));
    }
    let mut new_faces = Vec::new();
    let mut covered: HashSet<(u32, u32)> = HashSet::new();
    for &f in &faces {
        let mut nf = m.faces[f as usize].clone();
        for (a, b) in nf.edges() {
            covered.insert(ekey(a, b));
        }
        for v in &mut nf.verts {
            *v = map[v];
        }
        m.faces.push(nf);
        new_faces.push(m.faces.len() as u32 - 1);
    }
    for e in &edges {
        if !covered.contains(&ekey(e[0], e[1])) {
            m.loose_edges.push([map[&e[0]], map[&e[1]]]);
        }
    }
    let new: Vec<u32> = verts.iter().map(|v| map[v]).collect();
    if !new_faces.is_empty() && d.select_mode == SelectMode::Face {
        select_faces(d, &new_faces);
    } else {
        let set: BTreeSet<u32> = new.iter().copied().collect();
        select_verts_as_mode(d, &set, false);
    }
    after_topology(d);
    true
}

/// Inset of `faces` on a copy of `orig` (the modal I tool recomputes this
/// from the original on every pointer move).
pub fn inset_mesh(orig: &Mesh, faces: &[u32], thickness: f32, individual: bool) -> Mesh {
    let mut m = orig.clone();
    if thickness > 0.0 {
        if individual {
            ops::inset_faces_individual(&mut m, faces, thickness, 0.0);
        } else {
            ops::inset_region(&mut m, faces, thickness, 0.0);
        }
    }
    m
}

/// Bevel of `edges` on a copy of `orig` (the modal Ctrl+B tool): the mesh
/// and the new faces.
pub fn bevel_mesh(orig: &Mesh, edges: &[[u32; 2]], width: f32, segments: u32) -> (Mesh, Vec<u32>) {
    let mut m = orig.clone();
    let faces = if width > 0.0 { ops::bevel_edges(&mut m, edges, width, segments.max(1)) } else { Vec::new() };
    (m, faces)
}

/// Ctrl+R: cut `cuts` loops through the ring of `edge`; selects the new loop.
pub fn loop_cut(d: &mut Doc, edge: [u32; 2], cuts: u32) -> bool {
    let Some((_, mi)) = active_mesh(d) else { return false };
    d.push_undo("Loop Cut");
    let new = ops::loop_cut(&mut d.scene.meshes[mi], edge, cuts.max(1));
    if new.is_empty() {
        d.report = "No edge ring to cut".into();
        return false;
    }
    let mode = d.select_mode;
    if mode == SelectMode::Face {
        d.select_mode = SelectMode::Edge;
    }
    let set: BTreeSet<u32> = new.iter().copied().collect();
    select_verts_as_mode(d, &set, false);
    after_topology(d);
    true
}

/// The vertices a loop cut through `edge` would add, without changing the
/// document (for the hover preview): the cut mesh and the new vertices.
pub fn loop_cut_preview(m: &Mesh, edge: [u32; 2], cuts: u32) -> (Mesh, Vec<u32>) {
    let mut c = m.clone();
    let new = ops::loop_cut(&mut c, edge, cuts.max(1));
    (c, new)
}

/// Cut the mesh with a plane through world point `co` with world normal `no`
/// (the knife line); the cut vertices become the selection.
pub fn bisect(d: &mut Doc, co: Vec3, no: Vec3) -> bool {
    let Some((oi, mi)) = active_mesh(d) else { return false };
    let w = d.scene.world_matrix(oi);
    let inv = w.inverse();
    let co_l = inv.transform_point(co);
    let no_l = w.transpose().transform_vector(no).normalize();
    d.push_undo("Knife");
    let new = ops::bisect(&mut d.scene.meshes[mi], co_l, no_l, false, false, false);
    let set: BTreeSet<u32> = new.iter().copied().collect();
    if d.select_mode == SelectMode::Face {
        d.select_mode = SelectMode::Vertex;
    }
    select_verts_as_mode(d, &set, false);
    after_topology(d);
    true
}

/// F: an edge from two vertices, a face from three or more.
fn fill(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let verts = selected_verts(d);
    if verts.len() < 2 {
        d.report = "Select at least 2 vertices".into();
        return;
    }
    let m = &d.scene.meshes[mi];
    if verts.len() == 2 {
        let k = ekey(verts[0], verts[1]);
        if m.edges().iter().any(|e| (e[0], e[1]) == k) {
            d.report = "Already connected".into();
            return;
        }
        d.push_undo("Make Edge/Face");
        d.scene.meshes[mi].loose_edges.push([k.0, k.1]);
        flush(d);
        return;
    }
    // A boundary loop made of exactly the selection fills as that loop.
    let set: HashSet<u32> = verts.iter().copied().collect();
    let loops = ops::boundary_loops(m);
    let face: Vec<u32> = if let Some(lp) = loops.iter().find(|l| l.len() == set.len() && l.iter().all(|v| set.contains(v))) {
        lp.iter().rev().copied().collect()
    } else {
        // Otherwise order the vertices around their centre.
        let pts: Vec<Vec3> = verts.iter().map(|&v| m.positions[v as usize]).collect();
        let c = pts.iter().fold(Vec3::ZERO, |a, &b| a + b) / pts.len() as f32;
        let mut n = Vec3::ZERO;
        for i in 0..pts.len() {
            n += (pts[i] - c).cross(pts[(i + 1) % pts.len()] - c);
        }
        if n.length() < 1e-9 {
            n = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
        }
        let n = n.normalize();
        let x = if n == Vec3::ZERO { Vec3::X } else { n.any_orthonormal() };
        let y = n.cross(x);
        let mut order: Vec<(f32, u32)> = verts.iter().zip(&pts).map(|(&v, &p)| ((p - c).dot(y).atan2((p - c).dot(x)), v)).collect();
        order.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        order.into_iter().map(|o| o.1).collect()
    };
    d.push_undo("Make Edge/Face");
    let f = d.scene.meshes[mi].add_face(&face);
    d.scene.meshes[mi].loose_edges.retain(|e| !(set.contains(&e[0]) && set.contains(&e[1])));
    select_faces(d, &[f]);
    if d.select_mode != SelectMode::Face {
        flush(d);
    }
    after_topology(d);
}

fn merge(d: &mut Doc, kind: &str) {
    let Some((oi, mi)) = active_mesh(d) else { return };
    if kind == "BY_DISTANCE" {
        d.push_undo("Merge by Distance");
        let n = d.scene.meshes[mi].weld(1e-4);
        d.report = format!("Removed {n} vertice(s)");
        d.edit = EditSelection::default();
        after_topology(d);
        return;
    }
    let verts = selected_verts(d);
    if verts.len() < 2 {
        d.report = "Select at least 2 vertices".into();
        return;
    }
    d.push_undo("Merge");
    let before = d.scene.meshes[mi].positions.len();
    let mut kept = Vec::new();
    if kind == "COLLAPSE" {
        // Each connected island of the selection merges on its own.
        let m = &d.scene.meshes[mi];
        let sel: HashSet<u32> = verts.iter().copied().collect();
        let adj = adjacency(m);
        let mut seen = HashSet::new();
        let mut islands: Vec<Vec<u32>> = Vec::new();
        for &v in &verts {
            if !seen.insert(v) {
                continue;
            }
            let mut isl = vec![v];
            let mut st = vec![v];
            while let Some(x) = st.pop() {
                for &n in &adj[x as usize] {
                    if sel.contains(&n) && seen.insert(n) {
                        isl.push(n);
                        st.push(n);
                    }
                }
            }
            islands.push(isl);
        }
        // Merge islands from the highest indices down so earlier ones stay valid.
        let mut pos: Vec<Vec3> = Vec::new();
        for isl in &islands {
            let m = &d.scene.meshes[mi];
            pos.push(isl.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / isl.len() as f32);
        }
        // Snap every island to its centre, then weld them at a tiny distance.
        for (isl, p) in islands.iter().zip(&pos) {
            for &v in isl {
                d.scene.meshes[mi].positions[v as usize] = *p;
            }
        }
        d.scene.meshes[mi].weld(1e-6);
    } else if let Some(v) = ops::merge_at_center(&mut d.scene.meshes[mi], &verts) {
        if kind == "CURSOR" {
            let p = d.scene.world_matrix(oi).inverse().transform_point(d.cursor);
            d.scene.meshes[mi].positions[v as usize] = p;
        }
        kept.push(v);
    }
    let removed = before - d.scene.meshes[mi].positions.len();
    d.report = format!("Removed {removed} vertice(s)");
    d.edit = EditSelection::default();
    let set: BTreeSet<u32> = kept.into_iter().collect();
    let mode = d.select_mode;
    d.select_mode = SelectMode::Vertex;
    select_verts_as_mode(d, &set, false);
    d.select_mode = mode;
    after_topology(d);
}

fn delete(d: &mut Doc, kind: &str) {
    let Some((_, mi)) = active_mesh(d) else { return };
    if d.edit.verts.is_empty() {
        return;
    }
    d.push_undo("Delete");
    let verts = selected_verts(d);
    let faces = selected_faces(d);
    let edges: HashSet<(u32, u32)> = d.edit.edges.iter().copied().collect();
    let m = &mut d.scene.meshes[mi];
    match kind {
        "VERT" => ops::delete_verts(m, &verts),
        "FACE" => ops::delete_faces(m, &faces),
        "EDGE" | "EDGE_FACE" => {
            m.faces.retain(|f| !f.edges().any(|(a, b)| edges.contains(&ekey(a, b))));
            m.loose_edges.retain(|e| !edges.contains(&ekey(e[0], e[1])));
            if kind == "EDGE" {
                // Vertices left with nothing go too.
                let mut used = vec![false; m.positions.len()];
                for f in &m.faces {
                    for &v in &f.verts {
                        used[v as usize] = true;
                    }
                }
                for e in &m.loose_edges {
                    used[e[0] as usize] = true;
                    used[e[1] as usize] = true;
                }
                let gone: Vec<u32> = verts.iter().copied().filter(|&v| !used[v as usize]).collect();
                ops::delete_verts(m, &gone);
            }
        }
        "ONLY_FACE" => {
            let fs: HashSet<u32> = faces.iter().copied().collect();
            let mut keep_edges: HashSet<(u32, u32)> = HashSet::new();
            for (i, f) in m.faces.iter().enumerate() {
                if !fs.contains(&(i as u32)) {
                    keep_edges.extend(f.edges().map(|(a, b)| ekey(a, b)));
                }
            }
            let mut loose: Vec<[u32; 2]> = Vec::new();
            for &f in &faces {
                for (a, b) in m.faces[f as usize].edges() {
                    let k = ekey(a, b);
                    if !keep_edges.contains(&k) && !loose.contains(&[k.0, k.1]) {
                        loose.push([k.0, k.1]);
                    }
                }
            }
            let mut i = 0u32;
            m.faces.retain(|_| {
                let keep = !fs.contains(&i);
                i += 1;
                keep
            });
            m.loose_edges.extend(loose);
        }
        _ => {}
    }
    d.edit = EditSelection::default();
    d.edit_hidden.clear();
    after_topology(d);
}

fn separate(d: &mut Doc, kind: &str) {
    let Some((oi, mi)) = active_mesh(d) else { return };
    if kind != "SELECTED" {
        d.report = format!("Separate by {} is not available yet", kind.to_lowercase());
        return;
    }
    let faces = selected_faces(d);
    if faces.is_empty() {
        d.report = "Select faces to separate".into();
        return;
    }
    d.push_undo("Separate");
    let m = &d.scene.meshes[mi];
    let mut part = Mesh::new();
    let mut map: HashMap<u32, u32> = HashMap::new();
    for &f in &faces {
        let mut nf = m.faces[f as usize].clone();
        for v in &mut nf.verts {
            *v = *map.entry(*v).or_insert_with(|| {
                part.positions.push(m.positions[*v as usize]);
                part.positions.len() as u32 - 1
            });
        }
        part.faces.push(nf);
    }
    ops::delete_faces(&mut d.scene.meshes[mi], &faces);
    let src = d.scene.objects[oi].clone();
    let name = d.scene.mesh_names[mi].clone();
    let nm = d.scene.add_mesh(&name, part);
    let mut o = Object::new(&src.name, ObjectData::Mesh(nm));
    o.transform = src.transform;
    o.parent = src.parent;
    o.materials = src.materials.clone();
    o.modifiers = src.modifiers.clone();
    o.collection = src.collection;
    let ni = d.scene.add_object(o);
    if !d.scene.selected.contains(&ni) {
        d.scene.selected.push(ni);
    }
    d.edit = EditSelection::default();
    d.report = format!("Separated {}", d.scene.objects[ni].name);
    after_topology(d);
}

fn hide(d: &mut Doc, unselected: bool) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let n = d.scene.meshes[mi].positions.len() as u32;
    if unselected {
        let sel = d.edit.verts.clone();
        d.edit_hidden.extend((0..n).filter(|v| !sel.contains(v)));
    } else {
        let sel = d.edit.verts.clone();
        d.edit_hidden.extend(sel);
        d.edit = EditSelection::default();
    }
    flush(d);
}

fn reveal(d: &mut Doc) {
    let shown: BTreeSet<u32> = core::mem::take(&mut d.edit_hidden);
    select_verts_as_mode(d, &shown, true);
}

fn triangulate(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let faces: HashSet<u32> = d.edit.faces.iter().copied().collect();
    if faces.is_empty() {
        return;
    }
    d.push_undo("Triangulate Faces");
    let m = &mut d.scene.meshes[mi];
    let tris = m.triangulate();
    let mut first: HashSet<u32> = HashSet::new();
    let mut new_faces = Vec::new();
    for t in tris {
        if !faces.contains(&t.face) {
            continue;
        }
        let src = m.faces[t.face as usize].clone();
        let mut nf = src.clone();
        nf.verts = t.verts.to_vec();
        nf.uvs = if src.uvs.len() == src.verts.len() { t.corners.iter().map(|&c| src.uvs[c as usize]).collect() } else { Vec::new() };
        if first.insert(t.face) {
            new_faces.push((Some(t.face), nf));
        } else {
            new_faces.push((None, nf));
        }
    }
    let mut created = Vec::new();
    for (slot, nf) in new_faces {
        match slot {
            Some(f) => {
                m.faces[f as usize] = nf;
                created.push(f);
            }
            None => {
                m.faces.push(nf);
                created.push(m.faces.len() as u32 - 1);
            }
        }
    }
    select_faces(d, &created);
    if d.select_mode != SelectMode::Face {
        flush(d);
    }
    after_topology(d);
}

fn smooth_verts(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let verts = selected_verts(d);
    if verts.is_empty() {
        return;
    }
    d.push_undo("Smooth Vertices");
    let m = &mut d.scene.meshes[mi];
    let adj = adjacency(m);
    let old = m.positions.clone();
    for &v in &verts {
        let n = &adj[v as usize];
        if n.is_empty() {
            continue;
        }
        let avg = n.iter().fold(Vec3::ZERO, |a, &x| a + old[x as usize]) / n.len() as f32;
        m.positions[v as usize] = old[v as usize].lerp(avg, 0.5);
    }
    d.touch();
}

fn bridge(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let sel: HashSet<u32> = d.edit.verts.iter().copied().collect();
    let loops = ops::boundary_loops(&d.scene.meshes[mi]);
    let picks: Vec<u32> = loops.iter().filter_map(|l| l.iter().copied().find(|v| sel.contains(v))).collect();
    if picks.len() != 2 {
        d.report = "Select two edge loops on open boundaries".into();
        return;
    }
    d.push_undo("Bridge Edge Loops");
    match ops::bridge_edge_loops(&mut d.scene.meshes[mi], picks[0], picks[1]) {
        Ok(f) => {
            d.report = format!("Bridged with {} faces", f.len());
            after_topology(d);
        }
        Err(e) => d.report = e,
    }
}

/// Add a primitive into the edited mesh at the 3D cursor (Shift+A in Edit Mode).
fn add_primitive(d: &mut Doc, arg: &str) -> bool {
    let Some((oi, mi)) = active_mesh(d) else { return false };
    let mesh = match arg {
        "plane" => sex_3d::primitives::plane(2.0),
        "cube" => sex_3d::primitives::cube(2.0),
        "uv_sphere" => sex_3d::primitives::uv_sphere(32, 16, 1.0),
        "ico_sphere" => sex_3d::primitives::ico_sphere(2, 1.0),
        "cylinder" => sex_3d::primitives::cylinder(32, 1.0, 2.0),
        "cone" => sex_3d::primitives::cone(32, 1.0, 0.0, 2.0),
        "torus" => sex_3d::primitives::torus(48, 12, 1.0, 0.25),
        "grid" => sex_3d::primitives::grid(10, 10, 2.0),
        "monkey" => sex_3d::primitives::monkey(),
        _ => return false,
    };
    d.push_undo("Add Primitive");
    let mut mesh = mesh;
    let to_local = d.scene.world_matrix(oi).inverse() * sex_3d::Mat4::translation(d.cursor);
    mesh.transform(&to_local);
    let off = d.scene.meshes[mi].append(&mesh);
    let set: BTreeSet<u32> = (off..off + mesh.positions.len() as u32).collect();
    select_verts_as_mode(d, &set, false);
    after_topology(d);
    true
}

fn more_less(d: &mut Doc, more: bool) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let adj = adjacency(m);
    let sel = d.edit.verts.clone();
    let set: BTreeSet<u32> = if more {
        let mut s = sel.clone();
        for &v in &sel {
            s.extend(adj[v as usize].iter().copied().filter(|n| !d.edit_hidden.contains(n)));
        }
        s
    } else {
        sel.iter().copied().filter(|&v| adj[v as usize].iter().all(|n| sel.contains(n))).collect()
    };
    select_verts_as_mode(d, &set, false);
}

fn select_loose(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let mut used = vec![false; m.positions.len()];
    for f in &m.faces {
        for &v in &f.verts {
            used[v as usize] = true;
        }
    }
    let set: BTreeSet<u32> = (0..m.positions.len() as u32).filter(|&v| !used[v as usize]).collect();
    let mode = d.select_mode;
    d.select_mode = SelectMode::Vertex;
    select_verts_as_mode(d, &set, false);
    d.select_mode = mode;
}

fn edge_loops(d: &mut Doc) {
    let Some((_, mi)) = active_mesh(d) else { return };
    let m = &d.scene.meshes[mi];
    let he = m.half_edges();
    let mut verts: BTreeSet<u32> = BTreeSet::new();
    for &(a, b) in &d.edit.edges {
        let Some(h) = (0..he.half_edges.len() as u32).find(|&h| he.half_edges[h as usize].origin == a && he.dest(h) == b || he.half_edges[h as usize].origin == b && he.dest(h) == a) else { continue };
        for h in he.edge_loop(h, m) {
            verts.insert(he.half_edges[h as usize].origin);
            verts.insert(he.dest(h));
        }
    }
    let mode = d.select_mode;
    if mode == SelectMode::Face {
        d.select_mode = SelectMode::Edge;
    }
    // Choose the loop's edges.
    let mut s = EditSelection::default();
    for e in m.edges() {
        if verts.contains(&e[0]) && verts.contains(&e[1]) {
            s.edges.insert((e[0], e[1]));
        }
    }
    s.verts = verts;
    d.edit = s;
    flush(d);
}

/// Run a `mesh.*` operator. Returns false for ids this module does not know
/// (or that do not apply outside Edit Mode).
pub fn run(d: &mut Doc, op: &Op) -> bool {
    let id = op.id.as_str();
    if !id.starts_with("mesh.") {
        return false;
    }
    if d.mode != Mode::Edit {
        return false; // mesh.primitive_add adds an object in Object Mode
    }
    let arg = op.arg.as_str();
    if active_mesh(d).is_none() {
        d.report = "No mesh in Edit Mode".into();
        return true;
    }
    match id {
        "mesh.primitive_add" => {
            if !add_primitive(d, arg) {
                d.report = format!("Adding {arg} in Edit Mode is not available yet");
            }
        }
        "mesh.select_all" => select_all(d, if arg.is_empty() { "TOGGLE" } else { arg }),
        "mesh.select_mode" => {
            d.select_mode = match arg {
                "EDGE" => SelectMode::Edge,
                "FACE" => SelectMode::Face,
                _ => SelectMode::Vertex,
            };
            flush(d);
        }
        "mesh.select_linked" => {
            let Some((_, mi)) = active_mesh(d) else { return true };
            let seeds = selected_verts(d);
            let set = linked(&d.scene.meshes[mi], &seeds, &d.edit_hidden);
            select_verts_as_mode(d, &set, true);
        }
        "mesh.select_more" => more_less(d, true),
        "mesh.select_less" => more_less(d, false),
        "mesh.select_loose" => select_loose(d),
        "mesh.loop_multi_select" => edge_loops(d),
        "mesh.edge_face_add" => fill(d),
        "mesh.merge" => merge(d, if arg.is_empty() { "CENTER" } else { arg }),
        "mesh.remove_doubles" => merge(d, "BY_DISTANCE"),
        "mesh.delete" => delete(d, if arg.is_empty() { "VERT" } else { arg }),
        "mesh.delete_loose" => {
            d.push_undo("Delete Loose");
            let m = mesh_mut(d).expect("mesh");
            m.loose_edges.clear();
            let n = m.remove_unused_vertices();
            d.report = format!("Removed {n} vertice(s)");
            d.edit = EditSelection::default();
            after_topology(d);
        }
        "mesh.dissolve_limited" => {
            d.push_undo("Limited Dissolve");
            let m = mesh_mut(d).expect("mesh");
            let n = ops::dissolve_coplanar(m, 5f32.to_radians());
            d.report = format!("Dissolved {n}");
            d.edit = EditSelection::default();
            after_topology(d);
        }
        "mesh.normals_make_consistent" => {
            d.push_undo("Recalculate Normals");
            let m = mesh_mut(d).expect("mesh");
            ops::recalc_normals_outside(m);
            if arg == "INSIDE" {
                m.flip_normals();
            }
            d.touch();
        }
        "mesh.flip_normals" => {
            let faces = selected_faces(d);
            d.push_undo("Flip Normals");
            ops::flip_normals(mesh_mut(d).expect("mesh"), &faces);
            d.touch();
        }
        "mesh.duplicate" => {
            duplicate(d);
        }
        "mesh.extrude_faces_indiv" => {
            extrude_individual(d);
        }
        "mesh.separate" => separate(d, if arg.is_empty() { "SELECTED" } else { arg }),
        "mesh.hide" => hide(d, arg == "UNSELECTED"),
        "mesh.reveal" => reveal(d),
        "mesh.quads_convert_to_tris" => triangulate(d),
        "mesh.faces_shade_smooth" | "mesh.faces_shade_flat" => {
            let smooth = id == "mesh.faces_shade_smooth";
            let faces = selected_faces(d);
            d.push_undo(if smooth { "Shade Smooth" } else { "Shade Flat" });
            let m = mesh_mut(d).expect("mesh");
            for f in faces {
                m.faces[f as usize].smooth = smooth;
            }
            d.touch();
        }
        "mesh.vertices_smooth" => smooth_verts(d),
        "mesh.bridge_edge_loops" => bridge(d),
        "mesh.primitive_monkey_add" => {
            add_primitive(d, "monkey");
        }
        _ => return crate::edit_ops::run(d, op),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_doc() -> Doc {
        let mut d = Doc::default();
        d.run(&Op::with("object.mode_set", "EDIT"));
        d
    }

    #[test]
    fn enter_edit_selects_all_and_modes_flush() {
        let mut d = edit_doc();
        assert_eq!(d.edit.verts.len(), 8);
        assert_eq!(d.edit.faces.len(), 6);
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        assert!(d.edit.verts.is_empty());
        select_elem(&mut d, Some(Elem::Vert(0)), false);
        assert_eq!(d.edit.verts.len(), 1);
        assert!(d.edit.faces.is_empty());
        d.run(&Op::new("mesh.select_more"));
        assert_eq!(d.edit.verts.len(), 4);
        d.run(&Op::new("mesh.select_linked"));
        assert_eq!(d.edit.verts.len(), 8);
        d.run(&Op::with("mesh.select_mode", "FACE"));
        assert_eq!(d.edit.faces.len(), 6);
    }

    #[test]
    fn delete_merge_fill_separate_undo() {
        let mut d = edit_doc();
        let (_, mi) = active_mesh(&d).unwrap();
        d.run(&Op::with("mesh.select_mode", "FACE"));
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Face(0)), false);
        d.run(&Op::with("mesh.delete", "FACE"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 5);
        // Fill the hole back: select its four vertices and press F.
        d.run(&Op::with("mesh.select_mode", "VERT"));
        let lp = ops::boundary_loops(&d.scene.meshes[mi]).remove(0);
        d.edit = EditSelection::default();
        for v in lp {
            select_elem(&mut d, Some(Elem::Vert(v)), true);
        }
        d.run(&Op::new("mesh.edge_face_add"));
        assert_eq!(d.scene.meshes[mi].faces.len(), 6);
        assert!(ops::boundary_loops(&d.scene.meshes[mi]).is_empty());
        // Merge two vertices.
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(0)), false);
        select_elem(&mut d, Some(Elem::Vert(1)), true);
        d.run(&Op::with("mesh.merge", "CENTER"));
        assert_eq!(d.scene.meshes[mi].positions.len(), 7);
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.meshes[mi].positions.len(), 8);
        // Separate one face into its own object.
        let n = d.scene.objects.len();
        d.run(&Op::with("mesh.select_mode", "FACE"));
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Face(2)), false);
        d.run(&Op::with("mesh.separate", "SELECTED"));
        assert_eq!(d.scene.objects.len(), n + 1);
        assert_eq!(d.scene.meshes[mi].faces.len(), 5);
    }

    #[test]
    fn extrude_inset_bevel_loopcut() {
        let mut d = edit_doc();
        let (_, mi) = active_mesh(&d).unwrap();
        d.run(&Op::with("mesh.select_mode", "FACE"));
        select_elem(&mut d, Some(Elem::Face(0)), false);
        assert!(extrude(&mut d).unwrap().is_some());
        assert_eq!(d.scene.meshes[mi].faces.len(), 10);
        let m = inset_mesh(&d.scene.meshes[mi], &[0], 0.2, false);
        assert_eq!(m.faces.len(), 14);
        let (m, _) = bevel_mesh(&sex_3d::primitives::cube(2.0), &[[0, 1]], 0.2, 1);
        assert!(m.faces.len() > 6);
        let mut d = edit_doc();
        let e = d.scene.meshes[mi].edges()[0];
        assert!(loop_cut(&mut d, e, 1));
        assert_eq!(d.scene.meshes[mi].positions.len(), 12);
        assert_eq!(d.edit.verts.len(), 4);
    }

    #[test]
    fn hide_reveal_and_recalc() {
        let mut d = edit_doc();
        d.run(&Op::with("mesh.select_all", "DESELECT"));
        select_elem(&mut d, Some(Elem::Vert(3)), false);
        d.run(&Op::new("mesh.hide"));
        assert!(d.edit_hidden.contains(&3));
        d.run(&Op::with("mesh.select_all", "SELECT"));
        assert_eq!(d.edit.verts.len(), 7);
        d.run(&Op::new("mesh.reveal"));
        assert!(d.edit_hidden.is_empty());
        d.run(&Op::new("mesh.normals_make_consistent"));
        let (_, mi) = active_mesh(&d).unwrap();
        assert!(d.scene.meshes[mi].volume() > 0.0);
    }
}
