// SPDX-License-Identifier: GPL-3.0-only
//! Destructive modelling operations on a mesh: what Polyamour's edit mode
//! runs on the selected faces, edges or vertices.
//!
//! Every operation edits the [`Mesh`] in place and keeps it valid: faces stay
//! wound counter-clockwise seen from outside, closed meshes stay closed, and
//! UVs are carried along (interpolated inside the source face where a new
//! corner lies in it, otherwise guessed from neighbouring corners).
//! Selections are plain index lists: faces and vertices as `u32`, edges as
//! `[a, b]` vertex pairs in either order.
//!
//! Face indices of untouched faces are stable unless an operation documents
//! otherwise; vertex indices may be compacted by operations that remove
//! geometry.
use crate::math::{Quat, Vec2, Vec3};
use crate::mesh::{Face, Mesh, NONE};
use std::collections::{HashMap, HashSet};

/// Undirected edge key.
fn ekey(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// A length scale for tolerances: the bounding box diagonal (at least 1).
fn scale_of(m: &Mesh) -> f32 {
    let b = m.bounds();
    if b.is_empty() {
        1.0
    } else {
        b.size().length().max(1.0)
    }
}

/// Every directed edge (a, b) of every face, mapped to its face.
fn directed_edges(m: &Mesh) -> HashMap<(u32, u32), u32> {
    let mut map = HashMap::with_capacity(m.faces.len() * 4);
    for (fi, f) in m.faces.iter().enumerate() {
        for e in f.edges() {
            map.insert(e, fi as u32);
        }
    }
    map
}

/// UV at point `p` inside face `f`, extrapolated from the corner `c`: `p - P_c`
/// is expressed in the basis of the two face edges at that corner (least
/// squares) and the same combination is applied to the corner UVs. Exact for
/// points in the plane of a parallelogram-mapped face.
fn uv_in_face(m: &Mesh, f: usize, c: usize, p: Vec3) -> Vec2 {
    let face = &m.faces[f];
    let n = face.verts.len();
    if face.uvs.len() != n {
        return Vec2::ZERO;
    }
    let (inx, ipr) = ((c + 1) % n, (c + n - 1) % n);
    let o = m.positions[face.verts[c] as usize];
    let e1 = m.positions[face.verts[inx] as usize] - o;
    let e2 = m.positions[face.verts[ipr] as usize] - o;
    let d = p - o;
    let (a11, a12, a22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
    let det = a11 * a22 - a12 * a12;
    if det.abs() <= 1e-20 {
        return face.uvs[c];
    }
    let (b1, b2) = (d.dot(e1), d.dot(e2));
    let s = (b1 * a22 - b2 * a12) / det;
    let t = (a11 * b2 - a12 * b1) / det;
    let u0 = face.uvs[c];
    u0 + (face.uvs[inx] - u0) * s + (face.uvs[ipr] - u0) * t
}

/// Give UVs to faces that lack them when the rest of the mesh has some: each
/// corner takes the average UV its vertex has in the faces that do. Keeps
/// [`Mesh::has_uvs`] true after operations that add faces.
pub fn fill_missing_uvs(m: &mut Mesh) {
    if !m.faces.iter().any(|f| !f.uvs.is_empty()) {
        return;
    }
    let mut sum = vec![Vec2::ZERO; m.positions.len()];
    let mut cnt = vec![0u32; m.positions.len()];
    for f in &m.faces {
        if f.uvs.len() == f.verts.len() {
            for (&v, &uv) in f.verts.iter().zip(&f.uvs) {
                sum[v as usize] = sum[v as usize] + uv;
                cnt[v as usize] += 1;
            }
        }
    }
    for f in &mut m.faces {
        if f.uvs.len() != f.verts.len() {
            f.uvs = f.verts.iter().map(|&v| if cnt[v as usize] > 0 { sum[v as usize] * (1.0 / cnt[v as usize] as f32) } else { Vec2::ZERO }).collect();
        }
    }
}

/// Remove pairs of faces using the same vertex set (back-to-back duplicates,
/// as left between welded array copies or mirrored halves). Returns how many
/// faces were removed.
pub fn remove_doubled_faces(m: &mut Mesh) -> usize {
    let mut by_set: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        let mut k = f.verts.clone();
        k.sort_unstable();
        by_set.entry(k).or_default().push(fi);
    }
    let mut kill = vec![false; m.faces.len()];
    for list in by_set.values() {
        // Remove in pairs; an odd one out stays.
        for pair in list.chunks(2) {
            if pair.len() == 2 {
                kill[pair[0]] = true;
                kill[pair[1]] = true;
            }
        }
    }
    let before = m.faces.len();
    let mut i = 0;
    m.faces.retain(|_| {
        i += 1;
        !kill[i - 1]
    });
    before - m.faces.len()
}

// ---------------------------------------------------------------------------
// Extrude and inset

/// Result of detaching a face region from the rest of the mesh.
struct RegionSplit {
    /// Original vertex -> its copy, for vertices on the region's border.
    dup: HashMap<u32, u32>,
    /// Border edges (a, b) in the region faces' winding, original indices.
    border: Vec<(u32, u32)>,
    /// Indices of the side faces created along the border.
    sides: Vec<u32>,
    /// Every vertex the region faces use after the split.
    verts: Vec<u32>,
}

/// Detach `faces` along their border: border vertices are duplicated, the
/// region faces move onto the copies and a quad joins each border edge to its
/// copy. With the copies still in place the shape is unchanged; callers then
/// move them (extrude) or pull them inwards (inset).
fn split_region(m: &mut Mesh, faces: &[u32]) -> RegionSplit {
    let sel: HashSet<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len()).collect();
    let mut sel_sorted: Vec<u32> = sel.iter().copied().collect();
    sel_sorted.sort_unstable();
    let dir = directed_edges(m);
    let mut border = Vec::new();
    let mut border_corner = Vec::new();
    for &f in &sel_sorted {
        let face = &m.faces[f as usize];
        let n = face.verts.len();
        for c in 0..n {
            let (a, b) = (face.verts[c], face.verts[(c + 1) % n]);
            let inside = matches!(dir.get(&(b, a)), Some(g) if sel.contains(g));
            if !inside {
                border.push((a, b));
                border_corner.push((f, c));
            }
        }
    }
    let mut dup: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &border {
        for v in [a, b] {
            if !dup.contains_key(&v) {
                let p = m.positions[v as usize];
                let nv = m.add_vertex(p);
                dup.insert(v, nv);
            }
        }
    }
    let mut sides = Vec::with_capacity(border.len());
    for (&(a, b), &(f, c)) in border.iter().zip(&border_corner) {
        let face = &m.faces[f as usize];
        let n = face.verts.len();
        let uvs = if face.uvs.len() == n {
            let (ua, ub) = (face.uvs[c], face.uvs[(c + 1) % n]);
            vec![ua, ub, ub, ua]
        } else {
            Vec::new()
        };
        let (material, smooth) = (face.material, face.smooth);
        m.faces.push(Face { verts: vec![a, b, dup[&b], dup[&a]], uvs, material, smooth });
        sides.push((m.faces.len() - 1) as u32);
    }
    let mut verts = HashSet::new();
    for &f in &sel_sorted {
        for v in &mut m.faces[f as usize].verts {
            if let Some(&d) = dup.get(v) {
                *v = d;
            }
            verts.insert(*v);
        }
    }
    let mut verts: Vec<u32> = verts.into_iter().collect();
    verts.sort_unstable();
    RegionSplit { dup, border, sides, verts }
}

/// Area-weighted average normal of a set of faces.
fn region_normal(m: &Mesh, faces: &[u32]) -> Vec3 {
    faces.iter().fold(Vec3::ZERO, |acc, &f| acc + m.face_normal(f as usize) * m.face_area(f as usize)).normalize()
}

/// Region extrude: the selected faces are lifted by `distance` along their
/// average normal and joined to the rest of the mesh by side quads along the
/// region border, keeping connectivity (shared edges inside the region stay
/// shared). The selected faces keep their indices and become the top; returns
/// the new side faces.
pub fn extrude_faces(m: &mut Mesh, faces: &[u32], distance: f32) -> Vec<u32> {
    let n = region_normal(m, faces);
    let split = split_region(m, faces);
    for &v in &split.verts {
        m.positions[v as usize] += n * distance;
    }
    split.sides
}

/// Extrude each selected face on its own along its own normal.
pub fn extrude_faces_individual(m: &mut Mesh, faces: &[u32], distance: f32) -> Vec<u32> {
    let mut sides = Vec::new();
    for &f in faces {
        sides.extend(extrude_faces(m, &[f], distance));
    }
    sides
}

/// Where a corner moves when its two edges (unit directions `e_in` into the
/// corner and `e_out` out of it, in a face with normal `n`) are both offset
/// inwards by `t`.
fn inset_offset(n: Vec3, e_in: Vec3, e_out: Vec3, t: f32) -> Vec3 {
    let n1 = n.cross(e_in);
    let n2 = n.cross(e_out);
    let dir = (n1 + n2).normalize();
    if dir.length_squared() < 0.5 {
        return n1 * t;
    }
    let c = dir.dot(n1).max(0.25);
    dir * (t / c)
}

/// Region inset: the border of the selected region is pulled inwards by
/// `thickness` (within the faces) and the region lifted by `depth` along its
/// vertex normals; a ring of quads fills the gap. The selected faces keep
/// their indices; returns the new ring faces.
pub fn inset_region(m: &mut Mesh, faces: &[u32], thickness: f32, depth: f32) -> Vec<u32> {
    // Region normals per vertex, from the faces before the split.
    let mut vnorm: HashMap<u32, Vec3> = HashMap::new();
    for &f in faces {
        let n = m.face_normal(f as usize) * m.face_area(f as usize);
        for &v in &m.faces[f as usize].verts {
            *vnorm.entry(v).or_insert(Vec3::ZERO) += n;
        }
    }
    let split = split_region(m, faces);
    let mut out_of: HashMap<u32, u32> = HashMap::new();
    let mut into: HashMap<u32, u32> = HashMap::new();
    for &(a, b) in &split.border {
        out_of.entry(a).or_insert(b);
        into.entry(b).or_insert(a);
    }
    let mut moves = Vec::new();
    for (&v, &copy) in &split.dup {
        let (Some(&nx), Some(&pv)) = (out_of.get(&v), into.get(&v)) else { continue };
        let p = m.positions[v as usize];
        let n = vnorm.get(&v).copied().unwrap_or(Vec3::Z).normalize();
        let e_in = (p - m.positions[pv as usize]).normalize();
        let e_out = (m.positions[nx as usize] - p).normalize();
        moves.push((copy, inset_offset(n, e_in, e_out, thickness)));
    }
    for (copy, d) in moves {
        m.positions[copy as usize] += d;
    }
    if depth != 0.0 {
        let inv: HashMap<u32, u32> = split.dup.iter().map(|(&a, &b)| (b, a)).collect();
        for &v in &split.verts {
            let orig = inv.get(&v).copied().unwrap_or(v);
            let n = vnorm.get(&orig).copied().unwrap_or(Vec3::ZERO).normalize();
            m.positions[v as usize] += n * depth;
        }
    }
    split.sides
}

/// Inset each selected face on its own: a smaller copy of the face, offset
/// `thickness` from every edge and lifted `depth` along the face normal, with
/// a ring of quads around it. The face keeps its index; returns the ring.
pub fn inset_faces_individual(m: &mut Mesh, faces: &[u32], thickness: f32, depth: f32) -> Vec<u32> {
    let mut ring = Vec::new();
    for &f in faces {
        let fi = f as usize;
        if fi >= m.faces.len() {
            continue;
        }
        let n = m.face_normal(fi);
        let verts = m.faces[fi].verts.clone();
        let k = verts.len();
        let mut inner = Vec::with_capacity(k);
        let mut inner_uv = Vec::with_capacity(k);
        for c in 0..k {
            let p = m.positions[verts[c] as usize];
            let pp = m.positions[verts[(c + k - 1) % k] as usize];
            let pn = m.positions[verts[(c + 1) % k] as usize];
            let q = p + inset_offset(n, (p - pp).normalize(), (pn - p).normalize(), thickness) + n * depth;
            inner_uv.push(uv_in_face(m, fi, c, q));
            inner.push(q);
        }
        let first_new = m.positions.len() as u32;
        m.positions.extend_from_slice(&inner);
        let (material, smooth) = (m.faces[fi].material, m.faces[fi].smooth);
        let has_uv = m.faces[fi].uvs.len() == k;
        for c in 0..k {
            let d = (c + 1) % k;
            let uvs = if has_uv { vec![m.faces[fi].uvs[c], m.faces[fi].uvs[d], inner_uv[d], inner_uv[c]] } else { Vec::new() };
            m.faces.push(Face { verts: vec![verts[c], verts[d], first_new + d as u32, first_new + c as u32], uvs, material, smooth });
            ring.push((m.faces.len() - 1) as u32);
        }
        let face = &mut m.faces[fi];
        face.verts = (0..k as u32).map(|c| first_new + c).collect();
        if has_uv {
            face.uvs = inner_uv;
        }
    }
    ring
}

// ---------------------------------------------------------------------------
// Delete, merge, normals

/// Delete faces (and any vertices only they used). Vertex indices are compacted.
pub fn delete_faces(m: &mut Mesh, faces: &[u32]) {
    let kill: HashSet<u32> = faces.iter().copied().collect();
    let mut i = 0u32;
    m.faces.retain(|_| {
        i += 1;
        !kill.contains(&(i - 1))
    });
    m.remove_unused_vertices();
}

/// Delete vertices together with every face and edge using them. Other
/// vertices are kept (they may become loose); indices are compacted.
pub fn delete_verts(m: &mut Mesh, verts: &[u32]) {
    let mut dead = vec![false; m.positions.len()];
    for &v in verts {
        if let Some(d) = dead.get_mut(v as usize) {
            *d = true;
        }
    }
    m.faces.retain(|f| !f.verts.iter().any(|&v| dead[v as usize]));
    m.loose_edges.retain(|e| !dead[e[0] as usize] && !dead[e[1] as usize]);
    let mut remap = vec![0u32; m.positions.len()];
    let mut kept = Vec::with_capacity(m.positions.len());
    for (i, &p) in m.positions.iter().enumerate() {
        if !dead[i] {
            remap[i] = kept.len() as u32;
            kept.push(p);
        }
    }
    m.positions = kept;
    m.remap_vertices(&remap);
}

/// Merge the vertices into one at their centroid ("Merge at Center"). Faces
/// that collapse are removed; vertex indices are compacted. Returns the
/// merged vertex's new index.
pub fn merge_at_center(m: &mut Mesh, verts: &[u32]) -> Option<u32> {
    let verts: Vec<u32> = verts.iter().copied().filter(|&v| (v as usize) < m.positions.len()).collect();
    let &target = verts.first()?;
    let c = verts.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / verts.len() as f32;
    m.positions[target as usize] = c;
    let mut remap: Vec<u32> = (0..m.positions.len() as u32).collect();
    for &v in &verts {
        remap[v as usize] = target;
    }
    m.remap_vertices(&remap);
    // Compact, tracking where the target went.
    let mut used = vec![false; m.positions.len()];
    used[target as usize] = true;
    for f in &m.faces {
        for &v in &f.verts {
            used[v as usize] = true;
        }
    }
    for e in &m.loose_edges {
        used[e[0] as usize] = true;
        used[e[1] as usize] = true;
    }
    let mut remap = vec![0u32; m.positions.len()];
    let mut kept = Vec::new();
    for (i, &u) in used.iter().enumerate() {
        if u {
            remap[i] = kept.len() as u32;
            kept.push(m.positions[i]);
        }
    }
    m.positions = kept;
    m.remap_vertices(&remap);
    Some(remap[target as usize])
}

/// Reverse the winding of the given faces.
pub fn flip_normals(m: &mut Mesh, faces: &[u32]) {
    for &f in faces {
        if let Some(face) = m.faces.get_mut(f as usize) {
            face.verts.reverse();
            face.uvs.reverse();
        }
    }
}

/// Make normals consistent and outward ("Recalculate Outside"). Each connected
/// patch is walked breadth-first, flipping faces whose winding disagrees with
/// the neighbour they were reached from; then a patch whose faces point
/// inwards overall (negative `sum(area * n . (centre - centroid))`, which is
/// three times the signed volume for closed patches) is flipped as a whole.
/// Returns how many faces were flipped.
pub fn recalc_normals_outside(m: &mut Mesh) -> usize {
    let nf = m.faces.len();
    // Undirected edge -> (face, edge runs low-to-high in that face).
    let mut by_edge: HashMap<(u32, u32), Vec<(u32, bool)>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            by_edge.entry(ekey(a, b)).or_default().push((fi as u32, a < b));
        }
    }
    let mut flip = vec![false; nf];
    let mut seen = vec![false; nf];
    let mut queue = std::collections::VecDeque::new();
    for seed in 0..nf {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        queue.push_back(seed);
        let mut comp = Vec::new();
        while let Some(f) = queue.pop_front() {
            comp.push(f);
            for (a, b) in m.faces[f].edges() {
                let fwd = (a < b) != flip[f];
                for &(g, gfwd) in &by_edge[&ekey(a, b)] {
                    let g = g as usize;
                    if !seen[g] {
                        seen[g] = true;
                        // Consistent neighbours run the shared edge the other way.
                        flip[g] = fwd == gfwd;
                        queue.push_back(g);
                    }
                }
            }
        }
        // Orientation test for the patch.
        let mut centroid = Vec3::ZERO;
        let mut cnt = 0.0;
        for &f in &comp {
            for &v in &m.faces[f].verts {
                centroid += m.positions[v as usize];
                cnt += 1.0;
            }
        }
        centroid = centroid / cnt;
        let mut s = 0.0;
        for &f in &comp {
            let n = m.face_normal(f) * if flip[f] { -1.0 } else { 1.0 };
            s += m.face_area(f) * n.dot(m.face_center(f) - centroid);
        }
        if s < 0.0 {
            for &f in &comp {
                flip[f] = !flip[f];
            }
        }
    }
    let mut count = 0;
    for (f, &fl) in flip.iter().enumerate() {
        if fl {
            m.faces[f].verts.reverse();
            m.faces[f].uvs.reverse();
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// Holes and bridges

/// Boundary loops: chains of directed face edges (a, b) whose reverse is not
/// in any face, in the faces' winding. Filling a loop means adding the
/// reversed loop as a face.
pub fn boundary_loops(m: &Mesh) -> Vec<Vec<u32>> {
    let dir = directed_edges(m);
    let mut outgoing: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut order = Vec::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            if !dir.contains_key(&(b, a)) {
                outgoing.entry(a).or_default().push(b);
                order.push((a, b));
            }
        }
    }
    let mut used: HashSet<(u32, u32)> = HashSet::new();
    let mut loops = Vec::new();
    for (a0, b0) in order {
        if used.contains(&(a0, b0)) {
            continue;
        }
        used.insert((a0, b0));
        let mut lp = vec![a0];
        let mut cur = b0;
        let mut closed = false;
        while lp.len() <= dir.len() {
            if cur == a0 {
                closed = true;
                break;
            }
            lp.push(cur);
            let Some(next) = outgoing.get(&cur).and_then(|l| l.iter().copied().find(|&n| !used.contains(&(cur, n)))) else { break };
            used.insert((cur, next));
            cur = next;
        }
        if closed && lp.len() >= 3 {
            loops.push(lp);
        }
    }
    loops
}

/// Fill every boundary loop with an n-gon. Returns the new faces.
pub fn fill_holes(m: &mut Mesh) -> Vec<u32> {
    let loops = boundary_loops(m);
    let mut out = Vec::new();
    for lp in loops {
        out.push(m.add_face(&lp.into_iter().rev().collect::<Vec<_>>()));
    }
    fill_missing_uvs(m);
    out
}

/// Fill the boundary loop through vertex `v` with an n-gon.
pub fn fill_hole(m: &mut Mesh, v: u32) -> Option<u32> {
    let lp = boundary_loops(m).into_iter().find(|l| l.contains(&v))?;
    let f = m.add_face(&lp.into_iter().rev().collect::<Vec<_>>());
    fill_missing_uvs(m);
    Some(f)
}

/// Bridge two boundary loops with a band of faces. Each loop is given by any
/// vertex on it. Loops of equal length are joined by quads, starting from
/// the rotation that pairs the closest vertices; unequal loops get triangles
/// spread evenly. Returns the new faces, or an error if a vertex is not on a
/// boundary or both are on the same loop.
pub fn bridge_edge_loops(m: &mut Mesh, va: u32, vb: u32) -> Result<Vec<u32>, String> {
    let loops = boundary_loops(m);
    let la = loops.iter().position(|l| l.contains(&va)).ok_or("first vertex is not on a boundary loop")?;
    let lb = loops.iter().position(|l| l.contains(&vb)).ok_or("second vertex is not on a boundary loop")?;
    if la == lb {
        return Err("both vertices are on the same boundary loop".into());
    }
    // A runs in its boundary direction; C is B reversed, so a quad
    // [a(i+1), a(i), c(i), c(i+1)] runs both boundary edges backwards.
    let a = &loops[la];
    let mut c: Vec<u32> = loops[lb].iter().rev().copied().collect();
    let (n, k) = (a.len(), c.len());
    let p = |v: u32| m.positions[v as usize];
    let shift = if n == k {
        (0..k).min_by(|&s1, &s2| {
            let cost = |s: usize| (0..n).map(|i| p(a[i]).distance(p(c[(i + s) % k])).powi(2)).sum::<f32>();
            cost(s1).total_cmp(&cost(s2))
        })
    } else {
        (0..k).min_by(|&s1, &s2| p(a[0]).distance(p(c[s1])).total_cmp(&p(a[0]).distance(p(c[s2]))))
    }
    .unwrap_or(0);
    c.rotate_left(shift);
    let mut faces = Vec::new();
    if n == k {
        for i in 0..n {
            let j = (i + 1) % n;
            faces.push(m.add_face(&[a[j], a[i], c[i], c[j]]));
        }
    } else {
        let (mut i, mut j) = (0usize, 0usize);
        while i < n || j < k {
            // Advance whichever loop lags behind in normalised arc position.
            let advance_a = j >= k || (i < n && (i as f32 + 0.5) / n as f32 <= (j as f32 + 0.5) / k as f32);
            if advance_a {
                faces.push(m.add_face(&[a[(i + 1) % n], a[i % n], c[j % k]]));
                i += 1;
            } else {
                faces.push(m.add_face(&[a[i % n], c[j % k], c[(j + 1) % k]]));
                j += 1;
            }
        }
    }
    fill_missing_uvs(m);
    Ok(faces)
}

/// Spin (lathe) a chain of vertices `profile` around the axis through
/// `center`, sweeping `angle` radians in `steps` steps; `advance` moves each
/// step along the axis for a screw. A full turn without advance reuses the
/// first ring. Faces are wound so that a profile on +X with increasing Z,
/// spun counter-clockwise about +Z, faces outwards. Returns the new faces.
pub fn spin(m: &mut Mesh, profile: &[u32], center: Vec3, axis: Vec3, angle: f32, steps: u32, advance: f32) -> Vec<u32> {
    let steps = steps.max(1);
    let axis = axis.normalize();
    let full = (angle.abs() - core::f32::consts::TAU).abs() < 1e-4 && advance == 0.0;
    let mut rings: Vec<Vec<u32>> = vec![profile.to_vec()];
    let made = if full { steps - 1 } else { steps };
    for s in 1..=made {
        let t = s as f32 / steps as f32;
        let q = Quat::from_axis_angle(axis, angle * t);
        let ring = profile
            .iter()
            .map(|&v| {
                let p = m.positions[v as usize] - center;
                m.add_vertex(center + q.rotate(p) + axis * (advance * t))
            })
            .collect();
        rings.push(ring);
    }
    if full {
        rings.push(profile.to_vec());
    }
    let mut faces = Vec::new();
    for s in 0..rings.len() - 1 {
        let (r0, r1) = (&rings[s], &rings[s + 1]);
        for i in 0..profile.len().saturating_sub(1) {
            let f = [r0[i], r1[i], r1[i + 1], r0[i + 1]];
            faces.push(m.add_face(&f));
        }
    }
    fill_missing_uvs(m);
    faces
}

// ---------------------------------------------------------------------------
// Loop cut

/// Insert `cuts` evenly spaced edge loops across the ring of quads through
/// `edge` (Blender's Loop Cut). The ring is walked with the half-edge
/// structure from the edge's faces to the opposite edge of each quad until it
/// closes, reaches a boundary or reaches a non-quad, which then gets the new
/// vertices inserted into its edge. Returns the new vertices.
pub fn loop_cut(m: &mut Mesh, edge: [u32; 2], cuts: u32) -> Vec<u32> {
    let cuts = cuts.max(1);
    let he = m.half_edges();
    let hmap: HashMap<(u32, u32), u32> = (0..he.half_edges.len() as u32).map(|h| ((he.half_edges[h as usize].origin, he.dest(h)), h)).collect();
    let [a, b] = edge;
    let Some(&h0) = hmap.get(&(a, b)).or_else(|| hmap.get(&(b, a))) else { return Vec::new() };

    // Ring edges with an orientation (side 0 -> side 1), and ring quads with
    // the half-edge they were entered through.
    let mut orient: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
    orient.insert(ekey(a, b), (a, b));
    let mut ring_faces: HashMap<u32, u32> = HashMap::new();
    let mut starts = vec![h0];
    if he.half_edges[h0 as usize].twin != NONE {
        starts.push(he.half_edges[h0 as usize].twin);
    }
    for start in starts {
        let mut h = start;
        loop {
            let f = he.half_edges[h as usize].face;
            if m.faces[f as usize].verts.len() != 4 || ring_faces.contains_key(&f) {
                break;
            }
            ring_faces.insert(f, h);
            let (x, y) = (he.half_edges[h as usize].origin, he.dest(h));
            let opp = he.half_edges[he.half_edges[h as usize].next as usize].next;
            let (z, w) = (he.half_edges[opp as usize].origin, he.dest(opp));
            // Quad [x, y, z, w]: w faces x and z faces y across the quad.
            let o = orient[&ekey(x, y)];
            let oo = if o == (x, y) { (w, z) } else { (z, w) };
            orient.entry(ekey(z, w)).or_insert(oo);
            let t = he.half_edges[opp as usize].twin;
            if t == NONE {
                break;
            }
            h = t;
        }
    }

    // New vertices on every ring edge, stored in orientation order.
    let mut points: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut new_verts = Vec::new();
    let mut keys: Vec<_> = orient.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        let (s, e) = orient[&k];
        let (ps, pe) = (m.positions[s as usize], m.positions[e as usize]);
        let list: Vec<u32> = (1..=cuts).map(|i| m.add_vertex(ps.lerp(pe, i as f32 / (cuts + 1) as f32))).collect();
        new_verts.extend_from_slice(&list);
        points.insert(k, list);
    }
    // Points strictly inside edge u -> v, in that order, with their fractions.
    let along = |u: u32, v: u32| -> Option<Vec<(u32, f32)>> {
        let list = points.get(&ekey(u, v))?;
        let fwd = orient[&ekey(u, v)].0 == u;
        let n = list.len();
        Some(
            (0..n)
                .map(|i| {
                    let idx = if fwd { i } else { n - 1 - i };
                    (list[idx], (i + 1) as f32 / (n + 1) as f32)
                })
                .collect(),
        )
    };

    let old = std::mem::take(&mut m.faces);
    for (fi, f) in old.into_iter().enumerate() {
        let n = f.verts.len();
        let has_uv = f.uvs.len() == n;
        let uv = |c: usize| if has_uv { f.uvs[c % n] } else { Vec2::ZERO };
        if let Some(&h) = ring_faces.get(&(fi as u32)) {
            let c = (h - he.face_start[fi]) as usize;
            let (x, y, z, w) = (f.verts[c], f.verts[(c + 1) % 4], f.verts[(c + 2) % 4], f.verts[(c + 3) % 4]);
            let (ux, uy, uz, uw) = (uv(c), uv(c + 1), uv(c + 2), uv(c + 3));
            let mut left = vec![(x, ux)];
            left.extend(along(x, y).unwrap_or_default().into_iter().map(|(v, t)| (v, ux.lerp(uy, t))));
            left.push((y, uy));
            let mut right = vec![(w, uw)];
            right.extend(along(w, z).unwrap_or_default().into_iter().map(|(v, t)| (v, uw.lerp(uz, t))));
            right.push((z, uz));
            for k in 0..left.len().min(right.len()) - 1 {
                let corners = [left[k], left[k + 1], right[k + 1], right[k]];
                m.faces.push(Face {
                    verts: corners.iter().map(|c| c.0).collect(),
                    uvs: if has_uv { corners.iter().map(|c| c.1).collect() } else { Vec::new() },
                    material: f.material,
                    smooth: f.smooth,
                });
            }
        } else {
            let mut nf = Face { verts: Vec::with_capacity(n), uvs: Vec::new(), material: f.material, smooth: f.smooth };
            for c in 0..n {
                let (u, v) = (f.verts[c], f.verts[(c + 1) % n]);
                nf.verts.push(u);
                if has_uv {
                    nf.uvs.push(uv(c));
                }
                if let Some(pts) = along(u, v) {
                    for (p, t) in pts {
                        nf.verts.push(p);
                        if has_uv {
                            nf.uvs.push(uv(c).lerp(uv(c + 1), t));
                        }
                    }
                }
            }
            m.faces.push(nf);
        }
    }
    new_verts
}

// ---------------------------------------------------------------------------
// Bisect (knife along a plane)

/// Cut every face along the plane through `co` with normal `no` (Blender's
/// Bisect, the knife's plane mode). Faces crossing the plane are split;
/// `clear_inner` / `clear_outer` delete the part behind / in front of the
/// plane, and `fill` closes the cut with n-gons when one side is cleared.
/// Faces crossing the plane more than twice (strongly concave) are split by
/// side, which keeps the mesh valid but may not match the exact cut.
/// Returns the vertices lying on the plane afterwards (the cut loop).
pub fn bisect(m: &mut Mesh, co: Vec3, no: Vec3, fill: bool, clear_inner: bool, clear_outer: bool) -> Vec<u32> {
    let n = no.normalize();
    let eps = 1e-5 * scale_of(m);
    let dist: Vec<f32> = m.positions.iter().map(|&p| (p - co).dot(n)).collect();
    let side = |d: f32| -> i8 {
        if d > eps {
            1
        } else if d < -eps {
            -1
        } else {
            0
        }
    };
    let mut sides: Vec<i8> = dist.iter().map(|&d| side(d)).collect();
    let mut cut_vert: HashMap<(u32, u32), u32> = HashMap::new();
    let old = std::mem::take(&mut m.faces);
    // (face, which side it ended up on: 1, -1 or 0 for faces in the plane)
    let mut pieces: Vec<(Face, i8)> = Vec::with_capacity(old.len());
    for f in old {
        let k = f.verts.len();
        let has_uv = f.uvs.len() == k;
        let mut poly: Vec<(u32, Vec2)> = Vec::with_capacity(k + 2);
        for c in 0..k {
            let (a, b) = (f.verts[c], f.verts[(c + 1) % k]);
            let ua = if has_uv { f.uvs[c] } else { Vec2::ZERO };
            poly.push((a, ua));
            if sides[a as usize] * sides[b as usize] == -1 {
                let (lo, hi) = ekey(a, b);
                let t = dist[lo as usize] / (dist[lo as usize] - dist[hi as usize]);
                let v = *cut_vert.entry((lo, hi)).or_insert_with(|| {
                    let p = m.positions[lo as usize].lerp(m.positions[hi as usize], t);
                    m.positions.push(p);
                    sides.push(0);
                    (m.positions.len() - 1) as u32
                });
                let ub = if has_uv { f.uvs[(c + 1) % k] } else { Vec2::ZERO };
                let ta = if a == lo { t } else { 1.0 - t };
                poly.push((v, ua.lerp(ub, ta)));
            }
        }
        let has_pos = poly.iter().any(|&(v, _)| sides[v as usize] > 0);
        let has_neg = poly.iter().any(|&(v, _)| sides[v as usize] < 0);
        let make = |keep: &dyn Fn(i8) -> bool| -> Face {
            let kept: Vec<&(u32, Vec2)> = poly.iter().filter(|&&(v, _)| keep(sides[v as usize])).collect();
            Face { verts: kept.iter().map(|c| c.0).collect(), uvs: if has_uv { kept.iter().map(|c| c.1).collect() } else { Vec::new() }, material: f.material, smooth: f.smooth }
        };
        match (has_pos, has_neg) {
            (true, true) => {
                pieces.push((make(&|s| s >= 0), 1));
                pieces.push((make(&|s| s <= 0), -1));
            }
            (true, false) => pieces.push((make(&|_| true), 1)),
            (false, true) => pieces.push((make(&|_| true), -1)),
            (false, false) => pieces.push((make(&|_| true), 0)),
        }
    }
    for (f, s) in pieces {
        if f.verts.len() < 3 || (clear_inner && s < 0) || (clear_outer && s > 0) {
            continue;
        }
        m.faces.push(f);
    }
    if fill && clear_inner != clear_outer {
        for lp in boundary_loops(m) {
            if lp.iter().all(|&v| sides[v as usize] == 0) {
                m.add_face(&lp.into_iter().rev().collect::<Vec<_>>());
            }
        }
        fill_missing_uvs(m);
    }
    m.remove_unused_vertices();
    (0..m.positions.len() as u32).filter(|&v| (m.positions[v as usize] - co).dot(n).abs() <= eps).collect()
}

// ---------------------------------------------------------------------------
// Bevel

/// Bevel `edges` by `width` with `segments` faces across each bevel.
///
/// Works on manifold edges whose two vertices are interior (closed fan of
/// faces); others in the selection are ignored. At a vertex with two or more
/// bevelled edges, each run of faces between two bevelled edges shares one
/// new vertex, placed `width` from both edges (inside the face when the run
/// is a single face, on the shared edge otherwise). At a vertex with a single
/// bevelled edge the two new vertices slide along the neighbouring edges, so
/// the bevel tapers out like Blender's. With several segments the profile is
/// a circular arc (rational quadratic Bezier). Vertex caps are filled with an
/// n-gon (one segment) or a fan around a point on the rounded corner.
/// Returns the new faces (bevel strips and caps).
pub fn bevel_edges(m: &mut Mesh, edges: &[[u32; 2]], width: f32, segments: u32) -> Vec<u32> {
    let segments = segments.max(1);
    if width <= 0.0 || edges.is_empty() {
        return Vec::new();
    }
    let he = m.half_edges();
    let hmap: HashMap<(u32, u32), u32> = (0..he.half_edges.len() as u32).map(|h| ((he.half_edges[h as usize].origin, he.dest(h)), h)).collect();
    let nv = m.positions.len();
    let mut corner_count = vec![0usize; nv];
    for f in &m.faces {
        for &v in &f.verts {
            corner_count[v as usize] += 1;
        }
    }
    // Ordered fan of outgoing half-edges around an interior vertex.
    let fan = |v: u32| -> Option<Vec<u32>> {
        let start = he.vertex_out[v as usize];
        if start == NONE {
            return None;
        }
        let mut out = vec![start];
        let mut h = start;
        loop {
            let t = he.half_edges[he.half_edges[h as usize].prev as usize].twin;
            if t == NONE {
                return None;
            }
            if t == start {
                break;
            }
            out.push(t);
            h = t;
            if out.len() > corner_count[v as usize] {
                return None;
            }
        }
        (out.len() == corner_count[v as usize] && out.len() >= 3).then_some(out)
    };

    // Selected edges that can be bevelled, and the fans of their vertices.
    let mut fans: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut sel: HashSet<(u32, u32)> = HashSet::new();
    let mut sel_list: Vec<(u32, u32)> = Vec::new();
    for &[a, b] in edges {
        let k = ekey(a, b);
        if a == b || sel.contains(&k) || !hmap.contains_key(&(a, b)) || !hmap.contains_key(&(b, a)) {
            continue;
        }
        let mut ok = true;
        for v in [a, b] {
            if !fans.contains_key(&v) {
                match fan(v) {
                    Some(f) => {
                        fans.insert(v, f);
                    }
                    None => ok = false,
                }
            }
        }
        if ok {
            sel.insert(k);
            sel_list.push(k);
        }
    }
    if sel_list.is_empty() {
        return Vec::new();
    }

    let orig_faces = m.faces.clone();
    let orig_normals: Vec<Vec3> = (0..m.faces.len()).map(|f| m.face_normal(f)).collect();
    // (face, original vertex) -> the vertices replacing that corner.
    let mut corner: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    // Normal of a face next to each sector vertex (for rounding caps).
    let mut bv_normal: HashMap<u32, Vec3> = HashMap::new();
    let mut profiles: HashMap<(u32, u32, u32), Vec<u32>> = HashMap::new();

    let mut verts: Vec<u32> = fans.keys().copied().collect();
    verts.sort_unstable();
    for &v in &verts {
        let hs = &fans[&v];
        let n = hs.len();
        let pv = m.positions[v as usize];
        let nbr: Vec<u32> = hs.iter().map(|&h| he.dest(h)).collect();
        let dirs: Vec<Vec3> = nbr.iter().map(|&u| m.positions[u as usize] - pv).collect();
        let unit: Vec<Vec3> = dirs.iter().map(|d| d.normalize()).collect();
        let lens: Vec<f32> = dirs.iter().map(|d| d.length()).collect();
        let face_of = |i: usize| he.half_edges[hs[i % n] as usize].face;
        let bev: Vec<usize> = (0..n).filter(|&i| sel.contains(&ekey(v, nbr[i]))).collect();
        // Distance along edge `j` that is `width` away from edge `i` (both
        // edges of one face), clamped to half of edge `j`.
        let slide = |i: usize, j: usize| -> f32 {
            let s = unit[i].cross(unit[j]).length().max(1e-4);
            (width / s).min(0.5 * lens[j])
        };
        if bev.len() >= 2 {
            for (bi, &s) in bev.iter().enumerate() {
                let e = bev[(bi + 1) % bev.len()];
                let count = (e + n - s) % n;
                let p = if count == 1 {
                    let (i, j) = (s, (s + 1) % n);
                    let sn = unit[i].cross(unit[j]).length();
                    if sn < 1e-4 {
                        // Straight continuation: offset perpendicular inside the face.
                        pv + orig_normals[face_of(s) as usize].cross(unit[i]) * width
                    } else {
                        let k = (width / sn).min(0.5 * lens[i].min(lens[j]));
                        pv + (unit[i] + unit[j]) * k
                    }
                } else {
                    let first = (s + 1) % n;
                    let last = (e + n - 1) % n;
                    let d1 = slide(s, first);
                    let d2 = slide(e, last);
                    if first == last {
                        pv + unit[first] * ((d1 + d2) * 0.5)
                    } else {
                        (pv + unit[first] * d1 + pv + unit[last] * d2) * 0.5
                    }
                };
                let bv = m.add_vertex(p);
                bv_normal.insert(bv, orig_normals[face_of(s) as usize]);
                for i in 0..count {
                    corner.insert((face_of(s + i), v), vec![bv]);
                }
            }
        } else {
            let b = bev[0];
            let (bn, bp) = ((b + 1) % n, (b + n - 1) % n);
            let sa = m.add_vertex(pv + unit[bn] * slide(b, bn));
            let sc = m.add_vertex(pv + unit[bp] * slide(b, bp));
            corner.insert((face_of(b), v), vec![sa]);
            corner.insert((face_of(bp), v), vec![sc]);
            // Faces F(b+1) .. F(b-2) keep v, with the slid vertices inserted
            // next to it; with valence 3 the one face loses v entirely.
            for step in 1..n - 1 {
                let i = (b + step) % n;
                let seq = if n == 3 {
                    profile_path(m, &mut profiles, v, sc, sa, unit[b], segments)
                } else {
                    let mut seq = Vec::new();
                    if (i + 1) % n == bp {
                        seq.push(sc);
                    }
                    seq.push(v);
                    if i == bn {
                        seq.push(sa);
                    }
                    seq
                };
                corner.insert((face_of(i), v), seq);
            }
        }
    }

    // Rebuild the original faces with their corners replaced.
    let mut faces = Vec::with_capacity(orig_faces.len() + sel_list.len() * segments as usize);
    for (fi, f) in orig_faces.iter().enumerate() {
        let k = f.verts.len();
        let has_uv = f.uvs.len() == k;
        let mut nf = Face { verts: Vec::with_capacity(k + 2), uvs: Vec::new(), material: f.material, smooth: f.smooth };
        for c in 0..k {
            let v = f.verts[c];
            match corner.get(&(fi as u32, v)) {
                Some(seq) => {
                    for &x in seq {
                        nf.verts.push(x);
                        if has_uv {
                            nf.uvs.push(if x == v { f.uvs[c] } else { uv_in_face(m, fi, c, m.positions[x as usize]) });
                        }
                    }
                }
                None => {
                    nf.verts.push(v);
                    if has_uv {
                        nf.uvs.push(f.uvs[c]);
                    }
                }
            }
        }
        faces.push(nf);
    }

    // Bevel strips.
    let mut new_faces = Vec::new();
    for &(v, w) in &sel_list {
        let (hl, hr) = (hmap[&(v, w)], hmap[&(w, v)]);
        let (fl, fr) = (he.half_edges[hl as usize].face, he.half_edges[hr as usize].face);
        let cv = |f: u32, x: u32| corner[&(f, x)][0];
        let d = (m.positions[w as usize] - m.positions[v as usize]).normalize();
        let (lv, rv, lw, rw) = (cv(fl, v), cv(fr, v), cv(fl, w), cv(fr, w));
        let p = profile_path(m, &mut profiles, v, lv, rv, d, segments);
        let q = profile_path(m, &mut profiles, w, lw, rw, -d, segments);
        let src = &orig_faces[fl as usize];
        for j in 0..p.len().min(q.len()) - 1 {
            faces.push(Face { verts: vec![q[j], p[j], p[j + 1], q[j + 1]], uvs: Vec::new(), material: src.material, smooth: src.smooth || segments > 1 });
            new_faces.push((faces.len() - 1) as u32);
        }
    }
    m.faces = faces;

    // Caps: holes made only of new edges.
    let orig_edges: HashSet<(u32, u32)> = orig_faces.iter().flat_map(|f| f.edges().map(|(a, b)| ekey(a, b)).collect::<Vec<_>>()).collect();
    for lp in boundary_loops(m) {
        let k = lp.len();
        if (0..k).any(|i| orig_edges.contains(&ekey(lp[i], lp[(i + 1) % k]))) {
            continue;
        }
        let rev: Vec<u32> = lp.iter().rev().copied().collect();
        let material = 0;
        if segments > 1 && k > 4 {
            let center = cap_center(m, &rev, &bv_normal, width);
            let c = m.add_vertex(center);
            for i in 0..k {
                m.faces.push(Face { verts: vec![rev[i], rev[(i + 1) % k], c], uvs: Vec::new(), material, smooth: true });
                new_faces.push((m.faces.len() - 1) as u32);
            }
        } else {
            m.faces.push(Face { verts: rev, uvs: Vec::new(), material, smooth: segments > 1 });
            new_faces.push((m.faces.len() - 1) as u32);
        }
    }
    fill_missing_uvs(m);
    m.remove_unused_vertices();
    new_faces
}

/// The vertex path from `a` to `b` (both included) across the bevel profile
/// at vertex `v`, creating the inner points the first time. `d` is the
/// direction of the bevelled edge leaving `v`. The arc is the rational
/// quadratic Bezier with control point on the edge, which is an exact circle
/// arc tangent to both faces.
fn profile_path(m: &mut Mesh, profiles: &mut HashMap<(u32, u32, u32), Vec<u32>>, v: u32, a: u32, b: u32, d: Vec3, segments: u32) -> Vec<u32> {
    if segments <= 1 || a == b {
        return vec![a, b];
    }
    let key = (v, a.min(b), a.max(b));
    if !profiles.contains_key(&key) {
        let (lo, hi) = (key.1, key.2);
        let pv = m.positions[v as usize];
        let (pa, pb) = (m.positions[lo as usize], m.positions[hi as usize]);
        let c = pv + d * (((pa - pv).dot(d) + (pb - pv).dot(d)) * 0.5);
        let (ca, cb) = ((pa - c).normalize(), (pb - c).normalize());
        let theta = ca.dot(cb).clamp(-1.0, 1.0).acos();
        let wgt = (theta * 0.5).sin();
        let inner: Vec<u32> = (1..segments)
            .map(|i| {
                let t = i as f32 / segments as f32;
                let (b0, b1, b2) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t) * wgt, t * t);
                m.add_vertex((pa * b0 + c * b1 + pb * b2) / (b0 + b1 + b2))
            })
            .collect();
        profiles.insert(key, inner);
    }
    let inner = &profiles[&key];
    let mut out = vec![a];
    if a == key.1 {
        out.extend(inner.iter().copied());
    } else {
        out.extend(inner.iter().rev().copied());
    }
    out.push(b);
    out
}

/// A point for the middle of a rounded vertex cap: the estimated centre of
/// the rounding sphere (each sector vertex minus `width` along its face
/// normal) pushed out to the cap's average radius, towards the loop centroid.
fn cap_center(m: &Mesh, lp: &[u32], normals: &HashMap<u32, Vec3>, width: f32) -> Vec3 {
    let centroid = lp.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / lp.len() as f32;
    let est: Vec<Vec3> = lp.iter().filter_map(|v| normals.get(v).map(|n| m.positions[*v as usize] - *n * width)).collect();
    if est.is_empty() {
        return centroid;
    }
    let o = est.iter().fold(Vec3::ZERO, |a, &b| a + b) / est.len() as f32;
    let r = lp.iter().map(|&v| m.positions[v as usize].distance(o)).sum::<f32>() / lp.len() as f32;
    let dir = (centroid - o).normalize();
    if dir.length_squared() < 0.5 {
        centroid
    } else {
        o + dir * r
    }
}

// ---------------------------------------------------------------------------
// Limited dissolve

/// Merge adjacent faces that lie in one plane (normals within `angle`
/// radians, same material) into single n-gons, then remove vertices left in
/// the middle of a straight edge between two faces. Regions that would form
/// a face with a hole are left alone. Returns how many faces were removed.
pub fn dissolve_coplanar(m: &mut Mesh, angle: f32) -> usize {
    let nf = m.faces.len();
    let before = nf;
    let normals: Vec<Vec3> = (0..nf).map(|f| m.face_normal(f)).collect();
    let scale = scale_of(m);
    let cos_lim = angle.cos();
    let mut by_edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            by_edge.entry(ekey(a, b)).or_default().push(fi as u32);
        }
    }
    // Union-find over faces.
    let mut parent: Vec<u32> = (0..nf as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    let mut edge_keys: Vec<_> = by_edge.keys().copied().collect();
    edge_keys.sort_unstable();
    for k in edge_keys {
        let fs = &by_edge[&k];
        if fs.len() != 2 {
            continue;
        }
        let (f, g) = (fs[0] as usize, fs[1] as usize);
        if m.faces[f].material != m.faces[g].material || normals[f].dot(normals[g]) < cos_lim {
            continue;
        }
        let off = (m.face_center(g) - m.face_center(f)).dot(normals[f]).abs();
        if off > 1e-5 * scale {
            continue;
        }
        let (rf, rg) = (find(&mut parent, f as u32), find(&mut parent, g as u32));
        if rf != rg {
            parent[rf as usize] = rg;
        }
    }
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for f in 0..nf as u32 {
        let r = find(&mut parent, f);
        groups.entry(r).or_default().push(f);
    }
    let mut remove = vec![false; nf];
    let mut added = Vec::new();
    let mut roots: Vec<u32> = groups.keys().copied().collect();
    roots.sort_unstable();
    for r in roots {
        let fs = &groups[&r];
        if fs.len() < 2 {
            continue;
        }
        let mut directed: HashSet<(u32, u32)> = HashSet::new();
        let mut uv_of: HashMap<u32, Vec2> = HashMap::new();
        let mut has_uv = true;
        for &f in fs {
            let face = &m.faces[f as usize];
            directed.extend(face.edges());
            has_uv &= face.uvs.len() == face.verts.len();
            for (i, &v) in face.verts.iter().enumerate() {
                if let Some(&uv) = face.uvs.get(i) {
                    uv_of.entry(v).or_insert(uv);
                }
            }
        }
        let mut next: HashMap<u32, u32> = HashMap::new();
        let mut pinched = false;
        for &(a, b) in &directed {
            if !directed.contains(&(b, a)) && next.insert(a, b).is_some() {
                pinched = true;
            }
        }
        if pinched || next.is_empty() {
            continue;
        }
        let &start = next.keys().min().unwrap();
        let mut lp = vec![start];
        let mut cur = next[&start];
        while cur != start && lp.len() <= next.len() {
            lp.push(cur);
            match next.get(&cur) {
                Some(&n) => cur = n,
                None => break,
            }
        }
        if cur != start || lp.len() != next.len() {
            continue; // several loops: the region has a hole
        }
        let src = &m.faces[fs[0] as usize];
        let uvs = if has_uv { lp.iter().map(|v| uv_of[v]).collect() } else { Vec::new() };
        added.push(Face { verts: lp, uvs, material: src.material, smooth: src.smooth });
        for &f in fs {
            remove[f as usize] = true;
        }
    }
    let mut i = 0;
    m.faces.retain(|_| {
        i += 1;
        !remove[i - 1]
    });
    m.faces.extend(added);
    dissolve_straight_vertices(m);
    m.remove_unused_vertices();
    before - m.faces.len()
}

/// Remove vertices that sit in the middle of a straight edge between exactly
/// two faces (valence 2), as left behind by dissolving and by booleans.
fn dissolve_straight_vertices(m: &mut Mesh) {
    let nv = m.positions.len();
    let mut uses = vec![0u32; nv];
    let mut nbrs: Vec<[u32; 2]> = vec![[NONE; 2]; nv];
    let mut nbr_count = vec![0u32; nv];
    for f in &m.faces {
        for &v in &f.verts {
            uses[v as usize] += 1;
        }
    }
    for [a, b] in m.edges() {
        for (x, y) in [(a, b), (b, a)] {
            let c = nbr_count[x as usize];
            if c < 2 {
                nbrs[x as usize][c as usize] = y;
            }
            nbr_count[x as usize] += 1;
        }
    }
    let tol = 1e-6 * scale_of(m);
    let removable: Vec<bool> = (0..nv)
        .map(|v| {
            if uses[v] != 2 || nbr_count[v] != 2 {
                return false;
            }
            let p = m.positions[v];
            let (a, b) = (m.positions[nbrs[v][0] as usize], m.positions[nbrs[v][1] as usize]);
            let ab = b - a;
            let len = ab.length();
            len > 0.0 && (p - a).cross(ab).length() / len <= tol && (p - a).dot(b - p) > 0.0
        })
        .collect();
    for f in &mut m.faces {
        let k = f.verts.len();
        let drop = f.verts.iter().filter(|&&v| removable[v as usize]).count();
        if drop == 0 || k - drop < 3 {
            continue;
        }
        let has_uv = f.uvs.len() == k;
        let mut verts = Vec::with_capacity(k - drop);
        let mut uvs = Vec::new();
        for i in 0..k {
            if !removable[f.verts[i] as usize] {
                verts.push(f.verts[i]);
                if has_uv {
                    uvs.push(f.uvs[i]);
                }
            }
        }
        f.verts = verts;
        f.uvs = uvs;
    }
}

// ---------------------------------------------------------------------------
// Booleans

/// Which boolean to compute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    Union,
    Difference,
    Intersect,
}

type D3 = [f64; 3];

fn d_sub(a: D3, b: D3) -> D3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn d_dot(a: D3, b: D3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn d_lerp(a: D3, b: D3, t: f64) -> D3 {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

#[derive(Clone, Copy, Debug)]
struct BPlane {
    n: D3,
    w: f64,
}

#[derive(Clone, Copy, Debug)]
struct BVert {
    p: D3,
    uv: Vec2,
}

/// A convex polygon of a BSP solid.
#[derive(Clone, Debug)]
struct BPoly {
    verts: Vec<BVert>,
    plane: BPlane,
    material: u16,
    smooth: bool,
}

impl BPoly {
    /// Build from vertices, computing the plane (Newell); None if degenerate.
    fn new(verts: Vec<BVert>, material: u16, smooth: bool) -> Option<BPoly> {
        let mut n = [0.0f64; 3];
        let k = verts.len();
        for i in 0..k {
            let (p, q) = (verts[i].p, verts[(i + 1) % k].p);
            n[0] += (p[1] - q[1]) * (p[2] + q[2]);
            n[1] += (p[2] - q[2]) * (p[0] + q[0]);
            n[2] += (p[0] - q[0]) * (p[1] + q[1]);
        }
        let len = d_dot(n, n).sqrt();
        if k < 3 || len <= 1e-18 {
            return None;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let w = d_dot(n, verts[0].p);
        Some(BPoly { verts, plane: BPlane { n, w }, material, smooth })
    }
    fn flip(&mut self) {
        self.verts.reverse();
        self.plane = BPlane { n: [-self.plane.n[0], -self.plane.n[1], -self.plane.n[2]], w: -self.plane.w };
    }
}

/// Split `poly` by `plane` into the four output lists (csg.js style).
fn split_poly(plane: &BPlane, poly: BPoly, eps: f64, cop_front: &mut Vec<BPoly>, cop_back: &mut Vec<BPoly>, front: &mut Vec<BPoly>, back: &mut Vec<BPoly>) {
    const FRONT: u8 = 1;
    const BACK: u8 = 2;
    let mut kind = 0u8;
    let types: Vec<u8> = poly
        .verts
        .iter()
        .map(|v| {
            let t = d_dot(plane.n, v.p) - plane.w;
            let ty = if t < -eps {
                BACK
            } else if t > eps {
                FRONT
            } else {
                0
            };
            kind |= ty;
            ty
        })
        .collect();
    match kind {
        0 => {
            if d_dot(plane.n, poly.plane.n) > 0.0 {
                cop_front.push(poly)
            } else {
                cop_back.push(poly)
            }
        }
        FRONT => front.push(poly),
        BACK => back.push(poly),
        _ => {
            let k = poly.verts.len();
            let mut f = Vec::with_capacity(k + 1);
            let mut b = Vec::with_capacity(k + 1);
            for i in 0..k {
                let j = (i + 1) % k;
                let (ti, tj) = (types[i], types[j]);
                let (vi, vj) = (poly.verts[i], poly.verts[j]);
                if ti != BACK {
                    f.push(vi);
                }
                if ti != FRONT {
                    b.push(vi);
                }
                if (ti | tj) == FRONT | BACK {
                    let t = (plane.w - d_dot(plane.n, vi.p)) / d_dot(plane.n, d_sub(vj.p, vi.p));
                    let v = BVert { p: d_lerp(vi.p, vj.p, t), uv: vi.uv.lerp(vj.uv, t as f32) };
                    f.push(v);
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                front.push(BPoly { verts: f, plane: poly.plane, material: poly.material, smooth: poly.smooth });
            }
            if b.len() >= 3 {
                back.push(BPoly { verts: b, plane: poly.plane, material: poly.material, smooth: poly.smooth });
            }
        }
    }
}

#[derive(Default)]
struct BspNode {
    plane: Option<BPlane>,
    front: Option<usize>,
    back: Option<usize>,
    polys: Vec<BPoly>,
}

/// A BSP tree stored in an arena; every traversal is iterative so deep trees
/// cannot overflow the stack.
struct Bsp {
    nodes: Vec<BspNode>,
    eps: f64,
}

impl Bsp {
    fn new(polys: Vec<BPoly>, eps: f64) -> Bsp {
        let mut t = Bsp { nodes: vec![BspNode::default()], eps };
        t.build(polys);
        t
    }

    fn child(&mut self, node: usize, front: bool) -> usize {
        let existing = if front { self.nodes[node].front } else { self.nodes[node].back };
        if let Some(c) = existing {
            return c;
        }
        self.nodes.push(BspNode::default());
        let c = self.nodes.len() - 1;
        if front {
            self.nodes[node].front = Some(c);
        } else {
            self.nodes[node].back = Some(c);
        }
        c
    }

    fn build(&mut self, polys: Vec<BPoly>) {
        let mut stack = vec![(0usize, polys)];
        while let Some((ni, polys)) = stack.pop() {
            if polys.is_empty() {
                continue;
            }
            let plane = *self.nodes[ni].plane.get_or_insert(polys[0].plane);
            let (mut cf, mut cb, mut f, mut b) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for p in polys {
                split_poly(&plane, p, self.eps, &mut cf, &mut cb, &mut f, &mut b);
            }
            self.nodes[ni].polys.append(&mut cf);
            self.nodes[ni].polys.append(&mut cb);
            if !f.is_empty() {
                let c = self.child(ni, true);
                stack.push((c, f));
            }
            if !b.is_empty() {
                let c = self.child(ni, false);
                stack.push((c, b));
            }
        }
    }

    /// Remove the parts of `polys` inside this solid.
    fn clip_polygons(&self, polys: Vec<BPoly>) -> Vec<BPoly> {
        let mut out = Vec::new();
        let mut stack = vec![(0usize, polys)];
        while let Some((ni, polys)) = stack.pop() {
            let node = &self.nodes[ni];
            let Some(plane) = node.plane else {
                out.extend(polys);
                continue;
            };
            let (mut cf, mut cb, mut f, mut b) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for p in polys {
                split_poly(&plane, p, self.eps, &mut cf, &mut cb, &mut f, &mut b);
            }
            f.append(&mut cf);
            b.append(&mut cb);
            match node.front {
                Some(c) => stack.push((c, f)),
                None => out.extend(f),
            }
            if let Some(c) = node.back {
                stack.push((c, b));
            }
        }
        out
    }

    fn clip_to(&mut self, other: &Bsp) {
        for i in 0..self.nodes.len() {
            let polys = std::mem::take(&mut self.nodes[i].polys);
            self.nodes[i].polys = other.clip_polygons(polys);
        }
    }

    /// Turn the solid inside out.
    fn invert(&mut self) {
        for n in &mut self.nodes {
            for p in &mut n.polys {
                p.flip();
            }
            if let Some(pl) = &mut n.plane {
                *pl = BPlane { n: [-pl.n[0], -pl.n[1], -pl.n[2]], w: -pl.w };
            }
            std::mem::swap(&mut n.front, &mut n.back);
        }
    }

    fn all_polygons(&self) -> Vec<BPoly> {
        self.nodes.iter().flat_map(|n| n.polys.iter().cloned()).collect()
    }
}

/// Convert a mesh to convex BSP polygons: planar convex faces as they are,
/// anything else as its triangles.
fn mesh_to_polys(m: &Mesh, keep_uv: bool) -> Vec<BPoly> {
    let to_d = |p: Vec3| [p.x as f64, p.y as f64, p.z as f64];
    let tris = m.triangulate();
    let mut tri_start = vec![usize::MAX; m.faces.len()];
    for (i, t) in tris.iter().enumerate().rev() {
        tri_start[t.face as usize] = i;
    }
    let eps = 1e-5 * scale_of(m);
    let mut out = Vec::with_capacity(m.faces.len());
    for (fi, f) in m.faces.iter().enumerate() {
        let k = f.verts.len();
        if k < 3 {
            continue;
        }
        let uv = |c: usize| if keep_uv && f.uvs.len() == k { f.uvs[c] } else { Vec2::ZERO };
        let n = m.face_normal(fi);
        let p0 = m.positions[f.verts[0] as usize];
        let pts: Vec<Vec3> = f.verts.iter().map(|&v| m.positions[v as usize]).collect();
        let planar = pts.iter().all(|&p| (p - p0).dot(n).abs() <= eps);
        let convex = (0..k).all(|i| (pts[(i + 1) % k] - pts[i]).cross(pts[(i + 2) % k] - pts[(i + 1) % k]).dot(n) >= -1e-9);
        if k == 3 || (planar && convex) {
            let verts = (0..k).map(|c| BVert { p: to_d(pts[c]), uv: uv(c) }).collect();
            out.extend(BPoly::new(verts, f.material, f.smooth));
        } else if tri_start[fi] != usize::MAX {
            for t in tris[tri_start[fi]..].iter().take_while(|t| t.face == fi as u32) {
                let verts = t.corners.iter().map(|&c| BVert { p: to_d(pts[c as usize]), uv: uv(c as usize) }).collect();
                out.extend(BPoly::new(verts, f.material, f.smooth));
            }
        }
    }
    out
}

/// Insert vertices lying on the inside of an unmatched (boundary) edge into
/// that edge, so the polygon soup from the BSP becomes a connected surface.
fn fix_t_junctions(m: &mut Mesh, eps: f32) {
    for _ in 0..8 {
        let directed: HashSet<(u32, u32)> = m.faces.iter().flat_map(|f| f.edges().collect::<Vec<_>>()).collect();
        let mut bverts: HashSet<u32> = HashSet::new();
        let mut total_len = 0.0;
        let mut count = 0;
        for &(a, b) in &directed {
            if !directed.contains(&(b, a)) {
                bverts.insert(a);
                bverts.insert(b);
                total_len += m.positions[a as usize].distance(m.positions[b as usize]);
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        let cell = (total_len / count as f32).max(eps * 4.0);
        let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
        let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        for &v in &bverts {
            grid.entry(key(m.positions[v as usize])).or_default().push(v);
        }
        let mut changed = false;
        let pos = &m.positions;
        for f in &mut m.faces {
            let k = f.verts.len();
            let has_uv = f.uvs.len() == k;
            let mut verts = Vec::with_capacity(k);
            let mut uvs = Vec::new();
            for c in 0..k {
                let (a, b) = (f.verts[c], f.verts[(c + 1) % k]);
                verts.push(a);
                if has_uv {
                    uvs.push(f.uvs[c]);
                }
                if directed.contains(&(b, a)) {
                    continue;
                }
                let (pa, pb) = (pos[a as usize], pos[b as usize]);
                let ab = pb - pa;
                let l2 = ab.length_squared();
                if l2 <= 0.0 {
                    continue;
                }
                let (lo, hi) = (key(pa.min(pb) - Vec3::splat(eps)), key(pa.max(pb) + Vec3::splat(eps)));
                let mut found: Vec<(f32, u32)> = Vec::new();
                for x in lo.0..=hi.0 {
                    for y in lo.1..=hi.1 {
                        for z in lo.2..=hi.2 {
                            let Some(list) = grid.get(&(x, y, z)) else { continue };
                            for &v in list {
                                if v == a || v == b {
                                    continue;
                                }
                                let t = (pos[v as usize] - pa).dot(ab) / l2;
                                if t <= 1e-6 || t >= 1.0 - 1e-6 {
                                    continue;
                                }
                                if (pa + ab * t).distance(pos[v as usize]) <= eps {
                                    found.push((t, v));
                                }
                            }
                        }
                    }
                }
                found.sort_by(|x, y| x.0.total_cmp(&y.0));
                found.dedup_by_key(|x| x.1);
                for (t, v) in found {
                    verts.push(v);
                    if has_uv {
                        uvs.push(f.uvs[c].lerp(f.uvs[(c + 1) % k], t));
                    }
                    changed = true;
                }
            }
            f.verts = verts;
            f.uvs = uvs;
        }
        if !changed {
            return;
        }
    }
}

/// Boolean of two closed meshes with a BSP tree (in f64). The polygon soup
/// the tree produces is welded, its T-junctions are stitched and coplanar
/// fragments are merged back into n-gons, so closed manifold inputs in
/// general position give a closed manifold result. UVs are kept when both
/// inputs have them; materials and smooth flags come from the source faces.
pub fn boolean(a: &Mesh, b: &Mesh, op: BooleanOp) -> Mesh {
    let scale = scale_of(a).max(scale_of(b));
    let eps = 1e-6 * scale as f64;
    let keep_uv = a.has_uvs() && b.has_uvs();
    let mut ta = Bsp::new(mesh_to_polys(a, keep_uv), eps);
    let mut tb = Bsp::new(mesh_to_polys(b, keep_uv), eps);
    match op {
        BooleanOp::Union => {
            ta.clip_to(&tb);
            tb.clip_to(&ta);
            tb.invert();
            tb.clip_to(&ta);
            tb.invert();
            ta.build(tb.all_polygons());
        }
        BooleanOp::Difference => {
            ta.invert();
            ta.clip_to(&tb);
            tb.clip_to(&ta);
            tb.invert();
            tb.clip_to(&ta);
            tb.invert();
            ta.build(tb.all_polygons());
            ta.invert();
        }
        BooleanOp::Intersect => {
            ta.invert();
            tb.clip_to(&ta);
            tb.invert();
            ta.clip_to(&tb);
            tb.clip_to(&ta);
            ta.build(tb.all_polygons());
            ta.invert();
        }
    }
    let mut out = Mesh::new();
    for p in ta.all_polygons() {
        let first = out.positions.len() as u32;
        for v in &p.verts {
            out.positions.push(Vec3::new(v.p[0] as f32, v.p[1] as f32, v.p[2] as f32));
        }
        let k = p.verts.len() as u32;
        out.faces.push(Face {
            verts: (first..first + k).collect(),
            uvs: if keep_uv { p.verts.iter().map(|v| v.uv).collect() } else { Vec::new() },
            material: p.material,
            smooth: p.smooth,
        });
    }
    let weld = 1e-5 * scale;
    out.weld(weld);
    fix_t_junctions(&mut out, weld);
    dissolve_coplanar(&mut out, 1e-4);
    out.remove_unused_vertices();
    out
}
