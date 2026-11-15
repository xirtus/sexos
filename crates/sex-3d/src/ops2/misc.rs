// SPDX-License-Identifier: GPL-3.0-only
//! Poke, edge rotate/split, split, extrude variants, spin and screw,
//! symmetrize, make planar, boolean intersect, offset edge loops and
//! Laplacian smoothing.
use super::*;
use crate::math::Quat;
use crate::ops::{self, BooleanOp};

/// Poke Faces (Blender's defaults: offset 0, centre "median weighted"): a
/// vertex at each selected face's centre, weighted by the lengths of the
/// edges at each corner, joined to every corner by a fan of triangles.
/// Leaves the triangles selected.
pub fn poke(m: &mut Mesh, faces: &[u32], offset: f32) -> Selection {
    let mut out = Vec::new();
    for &f in faces {
        let fi = f as usize;
        if fi >= m.faces.len() {
            continue;
        }
        let src = m.faces[fi].clone();
        let cs = corners(&src);
        let n = cs.len();
        let p = |i: usize| m.positions[cs[i % n].0 as usize];
        let w: Vec<f32> = (0..n).map(|i| (p(i).distance(p(i + n - 1)) + p(i).distance(p(i + 1))) * 0.5).collect();
        let tw: f32 = w.iter().sum();
        let (mut c, mut uv) = (Vec3::ZERO, Vec2::ZERO);
        for i in 0..n {
            let k = if tw > 0.0 { w[i] / tw } else { 1.0 / n as f32 };
            c += p(i) * k;
            uv = uv + cs[i].1 * k;
        }
        let c = c + m.face_normal(fi) * offset;
        let cv = m.add_vertex(c);
        for i in 0..n {
            let tri = face_from(&src, &[cs[i], cs[(i + 1) % n], (cv, uv)]);
            if i == 0 {
                m.faces[fi] = tri;
                out.push(f);
            } else {
                m.faces.push(tri);
                out.push(m.faces.len() as u32 - 1);
            }
        }
    }
    Selection::faces(m, out)
}

/// Rotate Edge: each selected edge between two faces is removed and the two
/// faces re-split one corner further round, clockwise by default (Blender's
/// `use_ccw` off). Leaves the rotated edges selected.
pub fn edge_rotate(m: &mut Mesh, edges: &[[u32; 2]], ccw: bool) -> Selection {
    let mut dir = dir_map(m);
    let mut existing: HashSet<(u32, u32)> = m.edges().into_iter().map(|e| (e[0], e[1])).collect();
    let mut out = Vec::new();
    for e in edges {
        let (a, b) = (e[0], e[1]);
        let (Some(&f), Some(&g)) = (dir.get(&(a, b)), dir.get(&(b, a))) else { continue };
        if f == g {
            continue;
        }
        let rot = |face: &Face, first: u32| -> Vec<Corner> {
            let cs = corners(face);
            let i = cs.iter().position(|c| c.0 == first).unwrap_or(0);
            let n = cs.len();
            (0..n).map(|k| cs[(i + k) % n]).collect()
        };
        let fc = rot(&m.faces[f as usize], a); // a, b, x...
        let gc = rot(&m.faces[g as usize], b); // b, a, y...
        let mut l: Vec<Corner> = vec![fc[0]];
        l.extend_from_slice(&gc[2..]);
        let ib = l.len();
        l.push(fc[1]);
        l.extend_from_slice(&fc[2..]);
        let n = l.len();
        let (ia2, ib2) = if ccw { (1, (ib + 1) % n) } else { (n - 1, ib - 1) };
        let (na, nb) = (l[ia2].0, l[ib2].0);
        if na == nb || existing.contains(&ekey(na, nb)) || (ia2 + 1) % n == ib2 || (ib2 + 1) % n == ia2 {
            continue;
        }
        let arc = |from: usize, to: usize| -> Vec<Corner> {
            let mut o = Vec::new();
            let mut i = from;
            loop {
                o.push(l[i]);
                if i == to {
                    break;
                }
                i = (i + 1) % n;
            }
            o
        };
        for x in m.faces[f as usize].edges().chain(m.faces[g as usize].edges()).collect::<Vec<_>>() {
            dir.remove(&x);
        }
        let nf = face_from(&m.faces[f as usize], &arc(ia2, ib2));
        let ng = face_from(&m.faces[g as usize], &arc(ib2, ia2));
        m.faces[f as usize] = nf;
        m.faces[g as usize] = ng;
        for (fi, x) in [(f, m.faces[f as usize].edges().collect::<Vec<_>>()), (g, m.faces[g as usize].edges().collect::<Vec<_>>())] {
            for e in x {
                dir.insert(e, fi);
            }
        }
        existing.remove(&ekey(a, b));
        existing.insert(ekey(na, nb));
        out.push(ek(na, nb));
    }
    Selection::edges(out)
}

/// Edge Split: the faces on either side of each selected edge stop sharing
/// it. Every vertex on a selected edge is split into one copy per fan of
/// faces still joined through unselected edges. Leaves the split edges (both
/// copies) selected.
pub fn edge_split(m: &mut Mesh, edges: &[[u32; 2]]) -> Selection {
    let ef = edge_faces(m);
    let sel: HashSet<(u32, u32)> = edges.iter().map(|e| ekey(e[0], e[1])).filter(|k| ef.contains_key(k)).collect();
    // Corners of each selected edge in each of its faces.
    let mut records: Vec<(u32, usize, usize)> = Vec::new();
    for k in &sel {
        for &f in &ef[k] {
            let fv = &m.faces[f as usize].verts;
            if let (Some(i), Some(j)) = (fv.iter().position(|&v| v == k.0), fv.iter().position(|&v| v == k.1)) {
                records.push((f, i, j));
            }
        }
    }
    let vf = vert_faces(m);
    let adj = adjacency(m);
    let mut verts: Vec<u32> = sel.iter().flat_map(|&(a, b)| [a, b]).collect();
    verts.sort_unstable();
    verts.dedup();
    for v in verts {
        let fs = vf[v as usize].clone();
        if fs.len() < 2 {
            continue;
        }
        let local = |f: u32| fs.iter().position(|&x| x == f).unwrap() as u32;
        let mut dsu = Dsu::new(fs.len());
        for &u in &adj[v as usize] {
            let k = ekey(v, u);
            if sel.contains(&k) {
                continue;
            }
            if let Some(list) = ef.get(&k) {
                for w in list.windows(2) {
                    dsu.union(local(w[0]), local(w[1]));
                }
            }
        }
        let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
        for (i, &f) in fs.iter().enumerate() {
            let r = dsu.find(i as u32);
            groups.entry(r).or_default().push(f);
        }
        let mut roots: Vec<u32> = groups.keys().copied().collect();
        roots.sort_unstable();
        for r in roots.into_iter().skip(1) {
            let nv = m.add_vertex(m.positions[v as usize]);
            for &f in &groups[&r] {
                for x in &mut m.faces[f as usize].verts {
                    if *x == v {
                        *x = nv;
                    }
                }
            }
        }
    }
    let out: Vec<[u32; 2]> = records.iter().map(|&(f, i, j)| ek(m.faces[f as usize].verts[i], m.faces[f as usize].verts[j])).collect();
    Selection::edges(out)
}

/// Split (Y): the selected faces stop sharing vertices with the rest of the
/// mesh (their border vertices are duplicated). Leaves them selected.
pub fn split(m: &mut Mesh, faces: &[u32]) -> Selection {
    let sel: HashSet<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len()).collect();
    let mut outside: HashSet<u32> = HashSet::new();
    for (fi, f) in m.faces.iter().enumerate() {
        if !sel.contains(&(fi as u32)) {
            outside.extend(f.verts.iter().copied());
        }
    }
    outside.extend(m.loose_edges.iter().flat_map(|e| [e[0], e[1]]));
    let mut copy: HashMap<u32, u32> = HashMap::new();
    let mut sorted: Vec<u32> = sel.iter().copied().collect();
    sorted.sort_unstable();
    for &f in &sorted {
        for c in 0..m.faces[f as usize].verts.len() {
            let v = m.faces[f as usize].verts[c];
            if outside.contains(&v) {
                let nv = *copy.entry(v).or_insert_with(|| {
                    m.positions.push(m.positions[v as usize]);
                    m.positions.len() as u32 - 1
                });
                m.faces[f as usize].verts[c] = nv;
            }
        }
    }
    Selection::faces(m, sorted)
}

/// Extrude the given faces (as one region), wire edges and lone vertices
/// with zero offset, the way Blender's Extrude starts before the move:
/// region faces move onto copies of their border vertices with side quads
/// along the border; other edges get a quad to their copy; other vertices an
/// edge. Returns the extruded copy (moved faces, copied edges and vertices).
pub fn extrude_geometry(m: &mut Mesh, faces: &[u32], edges: &[[u32; 2]], verts: &[u32]) -> Selection {
    let faces: Vec<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len()).collect();
    let before: Vec<Vec<u32>> = faces.iter().map(|&f| m.faces[f as usize].verts.clone()).collect();
    let region_edges: HashSet<(u32, u32)> = before.iter().flat_map(|fv| (0..fv.len()).map(|i| ekey(fv[i], fv[(i + 1) % fv.len()])).collect::<Vec<_>>()).collect();
    let region_verts: HashSet<u32> = before.iter().flatten().copied().collect();
    let mut copy: HashMap<u32, u32> = HashMap::new();
    if !faces.is_empty() {
        ops::extrude_faces(m, &faces, 0.0);
        for (k, &f) in faces.iter().enumerate() {
            for (i, &v) in before[k].iter().enumerate() {
                let nv = m.faces[f as usize].verts[i];
                if nv != v {
                    copy.insert(v, nv);
                }
            }
        }
    }
    let dir = dir_map(m);
    let mut new_edges = Vec::new();
    let mut in_edge: HashSet<u32> = HashSet::new();
    for e in edges {
        let (a, b) = (e[0], e[1]);
        if a == b || region_edges.contains(&ekey(a, b)) {
            continue;
        }
        in_edge.insert(a);
        in_edge.insert(b);
        let mut dup = |v: u32| -> u32 {
            *copy.entry(v).or_insert_with(|| {
                m.positions.push(m.positions[v as usize]);
                m.positions.len() as u32 - 1
            })
        };
        let (na, nb) = (dup(a), dup(b));
        if dir.contains_key(&(a, b)) {
            m.add_face(&[b, a, na, nb]);
        } else {
            m.add_face(&[a, b, nb, na]);
        }
        m.loose_edges.retain(|le| ekey(le[0], le[1]) != ekey(a, b));
        new_edges.push(ek(na, nb));
    }
    for &v in verts {
        if region_verts.contains(&v) || in_edge.contains(&v) || v as usize >= m.positions.len() {
            continue;
        }
        let nv = m.add_vertex(m.positions[v as usize]);
        m.loose_edges.push([v, nv]);
        copy.insert(v, nv);
    }
    ops::fill_missing_uvs(m);
    let mut moved: Vec<u32> = faces.iter().flat_map(|&f| m.faces[f as usize].verts.clone()).collect();
    moved.extend(copy.values().copied());
    let mut s = Selection::faces(m, faces);
    s.verts.extend(moved);
    s.verts.sort_unstable();
    s.verts.dedup();
    s.edges.extend(new_edges);
    s.edges.sort_unstable();
    s.edges.dedup();
    s
}

/// Extrude Repeat (Blender's defaults: 10 steps): extrude the selection
/// `steps` times, moving each copy by `offset`. Returns the last copy.
pub fn extrude_repeat(m: &mut Mesh, faces: &[u32], edges: &[[u32; 2]], verts: &[u32], offset: Vec3, steps: u32) -> Selection {
    let mut cur = Selection { verts: verts.to_vec(), edges: edges.to_vec(), faces: faces.to_vec() };
    for _ in 0..steps {
        let next = extrude_geometry(m, &cur.faces, &cur.edges, &cur.verts);
        for &v in &next.verts {
            m.positions[v as usize] += offset;
        }
        cur = next;
    }
    cur
}

/// Extrude Manifold: a region extrude where a side wall that would lie in
/// the plane of the face next to it (that face is at right angles to the
/// region) is merged into that face instead, and the border vertices left
/// in a straight line along the extrusion are dissolved, so pushing or
/// pulling a face lengthens the shape without adding loops. Leaves the
/// region selected, ready to move along `normal`.
pub fn extrude_manifold(m: &mut Mesh, faces: &[u32]) -> Selection {
    let n = faces.iter().fold(Vec3::ZERO, |a, &f| a + m.face_normal(f as usize) * m.face_area(f as usize)).normalize();
    let region: HashSet<u32> = faces.iter().copied().collect();
    let first_side = m.faces.len() as u32;
    let sides = ops::extrude_faces(m, faces, 0.0);
    let dir = dir_map(m);
    let mut remove = vec![false; m.faces.len()];
    let mut border: Vec<u32> = Vec::new();
    for &s in &sides {
        let sv = m.faces[s as usize].verts.clone();
        let (a, b) = (sv[0], sv[1]);
        let Some(&g) = dir.get(&(b, a)) else { continue };
        if g >= first_side || region.contains(&g) || remove[g as usize] {
            continue;
        }
        if m.face_normal(g as usize).dot(n).abs() > 1e-3 {
            continue;
        }
        if let Some(lp) = region_outline(m, &[g, s]) {
            m.faces[g as usize] = face_from(&m.faces[g as usize], &lp);
            remove[s as usize] = true;
            border.push(a);
            border.push(b);
        }
    }
    retain_faces(m, &remove);
    // Border vertices now between the copy and one straight edge along n.
    let adj = adjacency(m);
    let straight: Vec<u32> = border
        .into_iter()
        .filter(|&v| {
            let nb = &adj[v as usize];
            nb.len() == 2 && nb.iter().all(|&u| {
                let d = m.positions[u as usize] - m.positions[v as usize];
                d.length() < 1e-9 || (d.normalize().dot(n)).abs() > 0.999
            })
        })
        .collect();
    dissolve_valence2(m, &straight);
    ops::fill_missing_uvs(m);
    let remap = compact(m);
    let _ = remap;
    Selection::faces(m, faces.to_vec())
}

/// Spin: extrude the selection `steps` times, rotating each copy by
/// `angle / steps` about `axis` through `center` (and moving it
/// `advance / steps` along the axis, for Screw). Wire edges are oriented so
/// the new faces point away from the axis. A full turn without advance
/// merges the last copy into the start (Blender's auto merge). Returns the
/// last copy.
pub fn spin(m: &mut Mesh, faces: &[u32], edges: &[[u32; 2]], verts: &[u32], center: Vec3, axis: Vec3, angle: f32, steps: u32, advance: f32) -> Selection {
    let steps = steps.max(1);
    let axis = axis.normalize();
    if axis == Vec3::ZERO {
        return Selection::default();
    }
    let q = Quat::from_axis_angle(axis, angle / steps as f32);
    let step_adv = axis * (advance / steps as f32);
    let face_edges: HashSet<(u32, u32)> = m.faces.iter().flat_map(|f| f.edges().map(|(a, b)| ekey(a, b)).collect::<Vec<_>>()).collect();
    // Orient wire edges for outward faces: quad [a, b, b', a'] has normal d x t.
    let oriented: Vec<[u32; 2]> = edges
        .iter()
        .map(|e| {
            if face_edges.contains(&ekey(e[0], e[1])) {
                return *e;
            }
            let (pa, pb) = (m.positions[e[0] as usize], m.positions[e[1] as usize]);
            let mid = (pa + pb) * 0.5 - center;
            let radial = mid - axis * mid.dot(axis);
            let t = axis.cross(radial) * angle.signum();
            let nrm = (pb - pa).cross(t);
            let s = if nrm.dot(radial).abs() > 1e-6 * nrm.length() * radial.length() { nrm.dot(radial) } else { nrm.dot(axis) * angle.signum() };
            if s < 0.0 {
                [e[1], e[0]]
            } else {
                *e
            }
        })
        .collect();
    let first_faces = m.faces.len();
    let mut start_verts: Vec<u32> = verts.to_vec();
    start_verts.extend(oriented.iter().flat_map(|e| [e[0], e[1]]));
    start_verts.extend(faces.iter().flat_map(|&f| m.faces[f as usize].verts.clone()));
    let region_dir = if faces.is_empty() {
        0.0
    } else {
        let c = faces.iter().fold(Vec3::ZERO, |a, &f| a + m.face_center(f as usize)) / faces.len() as f32;
        let nrm = faces.iter().fold(Vec3::ZERO, |a, &f| a + m.face_normal(f as usize));
        let motion = axis.cross(c - center) * angle.signum() + axis * advance;
        nrm.dot(motion)
    };
    let mut cur = Selection { verts: verts.to_vec(), edges: oriented, faces: faces.to_vec() };
    for _ in 0..steps {
        let next = extrude_geometry(m, &cur.faces, &cur.edges, &cur.verts);
        for &v in &next.verts {
            let p = m.positions[v as usize] - center;
            m.positions[v as usize] = center + q.rotate(p) + step_adv;
        }
        cur = next;
    }
    // Region extrude winds side walls for a move along the region normal.
    if region_dir < 0.0 {
        let region: HashSet<u32> = faces.iter().copied().collect();
        for f in first_faces..m.faces.len() {
            if !region.contains(&(f as u32)) {
                m.faces[f].verts.reverse();
                m.faces[f].uvs.reverse();
            }
        }
    }
    let full = (angle.abs() - core::f32::consts::TAU).abs() < 1e-4 && advance == 0.0;
    if full {
        let mut cand = start_verts.clone();
        cand.extend(cur.verts.iter().copied());
        let tol = 1e-5 * scale_of(m);
        weld_subset(m, &cand, tol);
        if !faces.is_empty() {
            // The region came back to where it started: it is now an inner wall.
            let region: HashSet<u32> = faces.iter().copied().collect();
            let remove: Vec<bool> = (0..m.faces.len()).map(|f| region.contains(&(f as u32))).collect();
            retain_faces(m, &remove);
        }
        compact(m);
        return Selection::default();
    }
    cur
}

/// Screw (Blender's defaults: 9 steps, 1 turn): spin an open chain of
/// selected edges about `axis` through `center`, advancing each turn by the
/// distance between the chain's two ends along the axis, and merging the
/// ends of consecutive turns.
pub fn screw(m: &mut Mesh, edges: &[[u32; 2]], center: Vec3, axis: Vec3, steps: u32, turns: u32) -> Result<Selection, String> {
    let mut nb: HashMap<u32, Vec<u32>> = HashMap::new();
    for e in edges {
        nb.entry(e[0]).or_default().push(e[1]);
        nb.entry(e[1]).or_default().push(e[0]);
    }
    let ends: Vec<u32> = nb.iter().filter(|(_, l)| l.len() == 1).map(|(&v, _)| v).collect();
    if ends.len() != 2 || nb.values().any(|l| l.len() > 2) {
        return Err("Screw needs one open chain of selected edges".into());
    }
    let axis = axis.normalize();
    let pitch = (m.positions[ends[1] as usize] - m.positions[ends[0] as usize]).dot(axis).abs();
    let turns = turns.max(1);
    let before = m.positions.len() as u32;
    let s = spin(m, &[], edges, &[], center, axis, core::f32::consts::TAU * turns as f32, steps.max(1) * turns, pitch * turns as f32);
    let mut cand: Vec<u32> = (before..m.positions.len() as u32).collect();
    cand.extend(nb.keys().copied());
    let tol = 1e-5 * scale_of(m);
    weld_subset(m, &cand, tol);
    let remap = compact(m);
    Ok(Selection::verts(remap_list(&s.verts, &remap)))
}

/// A mesh of just the given faces (vertices renumbered).
fn submesh(m: &Mesh, faces: &[u32]) -> Mesh {
    let mut out = Mesh::new();
    let mut map: HashMap<u32, u32> = HashMap::new();
    for &f in faces {
        let mut nf = m.faces[f as usize].clone();
        for v in &mut nf.verts {
            *v = *map.entry(*v).or_insert_with(|| {
                out.positions.push(m.positions[*v as usize]);
                out.positions.len() as u32 - 1
            });
        }
        out.faces.push(nf);
    }
    out
}

fn axis_vec(axis: usize) -> Vec3 {
    match axis {
        0 => Vec3::X,
        1 => Vec3::Y,
        _ => Vec3::Z,
    }
}
fn set_axis(p: &mut Vec3, axis: usize, v: f32) {
    match axis {
        0 => p.x = v,
        1 => p.y = v,
        _ => p.z = v,
    }
}

/// Symmetrize (Blender's default direction -X to +X): the selected faces
/// are cut at the mirror plane, the half on the `negative_to_positive`
/// side's far side is replaced by a mirrored copy of the other half, and the
/// halves are welded at the plane (and to the unselected mesh around them).
/// Leaves the symmetric faces selected.
pub fn symmetrize(m: &mut Mesh, faces: &[u32], axis: usize, negative_to_positive: bool, threshold: f32) -> Selection {
    let mut sel: Vec<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len()).collect();
    sel.sort_unstable();
    sel.dedup();
    if sel.is_empty() {
        return Selection::default();
    }
    let n = axis_vec(axis);
    let mut sub = submesh(m, &sel);
    ops::bisect(&mut sub, Vec3::ZERO, n, false, !negative_to_positive, negative_to_positive);
    for p in &mut sub.positions {
        if p[axis].abs() <= threshold {
            set_axis(p, axis, 0.0);
        }
    }
    let mut mir = sub.clone();
    for p in &mut mir.positions {
        let v = -p[axis];
        set_axis(p, axis, v);
    }
    mir.flip_normals();
    sub.append(&mir);
    let on_plane: Vec<u32> = (0..sub.positions.len() as u32).filter(|&v| sub.positions[v as usize][axis] == 0.0).collect();
    weld_subset(&mut sub, &on_plane, threshold.max(1e-6));
    ops::remove_doubled_faces(&mut sub);
    compact(&mut sub);
    let old_verts: Vec<u32> = sel.iter().flat_map(|&f| m.faces[f as usize].verts.clone()).collect();
    let remove: Vec<bool> = (0..m.faces.len()).map(|f| sel.binary_search(&(f as u32)).is_ok()).collect();
    retain_faces(m, &remove);
    let base = m.faces.len() as u32;
    let off = m.append(&sub);
    let new_faces: Vec<u32> = (base..m.faces.len() as u32).collect();
    let mut cand = old_verts;
    cand.extend(off..m.positions.len() as u32);
    weld_subset(m, &cand, threshold.max(1e-6 * scale_of(m)));
    compact(m);
    Selection::faces(m, new_faces)
}

/// Snap to Symmetry (Blender's defaults: -X to +X, threshold 0.05, factor
/// 0.5, centre on): each selected vertex on the source side finds the
/// selected vertex nearest its mirror image (within `threshold`); the pair
/// is moved to mirror each other exactly, mixed by `factor` (1 keeps the
/// source). Unpaired vertices within `threshold` of the plane snap onto it.
pub fn symmetry_snap(m: &mut Mesh, verts: &[u32], axis: usize, negative_to_positive: bool, threshold: f32, factor: f32, use_center: bool) -> Selection {
    let sel: Vec<u32> = verts.iter().copied().filter(|&v| (v as usize) < m.positions.len()).collect();
    let mirror = |p: Vec3| {
        let mut q = p;
        set_axis(&mut q, axis, -p[axis]);
        q
    };
    let src_side = |p: Vec3| if negative_to_positive { p[axis] < 0.0 } else { p[axis] > 0.0 };
    let mut paired: HashSet<u32> = HashSet::new();
    for &v in &sel {
        let pv = m.positions[v as usize];
        if !src_side(pv) || pv[axis].abs() <= threshold * 0.5 || paired.contains(&v) {
            continue;
        }
        let target = mirror(pv);
        let best = sel
            .iter()
            .copied()
            .filter(|&w| w != v && !paired.contains(&w) && !src_side(m.positions[w as usize]))
            .map(|w| (w, m.positions[w as usize].distance(target)))
            .filter(|&(_, d)| d <= threshold)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((w, _)) = best {
            let mixed = mirror(m.positions[w as usize]).lerp(pv, factor);
            m.positions[v as usize] = mixed;
            m.positions[w as usize] = mirror(mixed);
            paired.insert(v);
            paired.insert(w);
        }
    }
    if use_center {
        for &v in &sel {
            if !paired.contains(&v) && m.positions[v as usize][axis].abs() <= threshold {
                set_axis(&mut m.positions[v as usize], axis, 0.0);
            }
        }
    }
    Selection::verts(sel)
}

/// Make Planar Faces (Blender's defaults: factor 1, one iteration): each
/// selected face's vertices are projected onto its plane; vertices shared by
/// several faces take the average.
pub fn face_make_planar(m: &mut Mesh, faces: &[u32], factor: f32, repeat: u32) {
    for _ in 0..repeat.max(1) {
        let mut acc: HashMap<u32, (Vec3, f32)> = HashMap::new();
        for &f in faces {
            let fi = f as usize;
            if fi >= m.faces.len() || m.faces[fi].verts.len() < 4 {
                continue;
            }
            let (n, c) = (m.face_normal(fi), m.face_center(fi));
            for &v in &m.faces[fi].verts {
                let p = m.positions[v as usize];
                let e = acc.entry(v).or_insert((Vec3::ZERO, 0.0));
                e.0 += p - n * (p - c).dot(n);
                e.1 += 1.0;
            }
        }
        for (v, (s, c)) in acc {
            let p = m.positions[v as usize];
            m.positions[v as usize] = p.lerp(s / c, factor);
        }
    }
}

/// Intersect (Boolean) (Blender's default: Difference): the unselected
/// faces are combined with the selected ones as two solids, unselected
/// `op` selected. The mesh is replaced by the result (wire edges kept);
/// nothing stays selected.
pub fn intersect_boolean(m: &mut Mesh, faces: &[u32], op: BooleanOp) -> Result<(), String> {
    let sel: HashSet<u32> = faces.iter().copied().collect();
    let b_faces: Vec<u32> = (0..m.faces.len() as u32).filter(|f| sel.contains(f)).collect();
    let a_faces: Vec<u32> = (0..m.faces.len() as u32).filter(|f| !sel.contains(f)).collect();
    if a_faces.is_empty() || b_faces.is_empty() {
        return Err("Select the cutting faces; the unselected faces are cut".into());
    }
    let mut out = ops::boolean(&submesh(m, &a_faces), &submesh(m, &b_faces), op);
    for e in &m.loose_edges {
        let a = out.add_vertex(m.positions[e[0] as usize]);
        let b = out.add_vertex(m.positions[e[1] as usize]);
        out.loose_edges.push([a, b]);
    }
    *m = out;
    Ok(())
}

/// Merge the selected vertices at the position of `target` (Blender's
/// Merge At First / At Last). Returns the merged vertex.
pub fn merge_at_vertex(m: &mut Mesh, verts: &[u32], target: u32) -> Option<u32> {
    let p = *m.positions.get(target as usize)?;
    let v = ops::merge_at_center(m, verts)?;
    m.positions[v as usize] = p;
    Some(v)
}

/// Offset Edge Loops: a new edge loop either side of the selected edges,
/// a fraction `t` of the way along the edges leading away from them (the
/// selected loop stays). Each face next to the loop is cut in two; faces
/// further out gain the new vertex on their shared edge. Leaves the new
/// loops selected.
pub fn offset_edge_loops(m: &mut Mesh, edges: &[[u32; 2]], t: f32) -> Selection {
    let sel: HashSet<(u32, u32)> = edges.iter().map(|e| ekey(e[0], e[1])).collect();
    let mut points: HashMap<(u32, u32), u32> = HashMap::new();
    let mut new_edges = Vec::new();
    let mut point = |m: &mut Mesh, v: u32, u: u32| -> u32 {
        *points.entry((v, u)).or_insert_with(|| {
            let p = m.positions[v as usize].lerp(m.positions[u as usize], t);
            m.add_vertex(p)
        })
    };
    let nf = m.faces.len();
    let mut rungs: Vec<(u32, u32, u32)> = Vec::new();
    for fi in 0..nf {
        let f = m.faces[fi].clone();
        let n = f.verts.len();
        let hits: Vec<usize> = (0..n).filter(|&i| sel.contains(&ekey(f.verts[i], f.verts[(i + 1) % n]))).collect();
        if hits.len() != 1 {
            continue;
        }
        let i = hits[0];
        let (a, b) = (f.verts[i], f.verts[(i + 1) % n]);
        let (after_b, before_a) = (f.verts[(i + 2) % n], f.verts[(i + n - 1) % n]);
        let cs = corners(&f);
        let na = point(m, a, before_a);
        let nb = point(m, b, after_b);
        rungs.push((a, before_a, na));
        rungs.push((b, after_b, nb));
        let (ua, ub) = (cs[i].1, cs[(i + 1) % n].1);
        let (ua2, ub2) = (ua.lerp(cs[(i + n - 1) % n].1, t), ub.lerp(cs[(i + 2) % n].1, t));
        let p1 = vec![cs[i], cs[(i + 1) % n], (nb, ub2), (na, ua2)];
        let mut p2 = vec![(na, ua2), (nb, ub2)];
        for k in 2..n {
            p2.push(cs[(i + k) % n]);
        }
        m.faces[fi] = face_from(&f, &p1);
        m.faces.push(face_from(&f, &p2));
        new_edges.push(ek(na, nb));
    }
    // Faces further out share the cut edges.
    let mut done = HashSet::new();
    for (v, u, x) in rungs {
        if done.insert((v, u)) {
            insert_edge_vertex(m, v, u, x, t);
        }
    }
    Selection::edges(new_edges)
}

/// Laplacian Smooth (Blender's defaults: 1 repeat, lambda 1, border lambda
/// 5e-5, volume preserved): implicit smoothing with cotangent weights solved
/// by Gauss-Seidel; unselected vertices stay fixed, boundary vertices use
/// the border lambda, and the selection is rescaled about its centre to keep
/// the mesh volume.
pub fn smooth_laplacian(m: &mut Mesh, verts: &[u32], repeat: u32, lambda: f32, lambda_border: f32, preserve_volume: bool) {
    let nv = m.positions.len();
    let sel: Vec<u32> = verts.iter().copied().filter(|&v| (v as usize) < nv).collect();
    if sel.is_empty() {
        return;
    }
    let v0 = m.volume();
    let ef = edge_faces(m);
    let boundary: HashSet<u32> = ef.iter().filter(|(_, f)| f.len() == 1).flat_map(|(k, _)| [k.0, k.1]).collect();
    for _ in 0..repeat.max(1) {
        let mut w: HashMap<(u32, u32), f32> = HashMap::new();
        for t in m.triangulate() {
            let p = t.verts.map(|v| m.positions[v as usize]);
            for k in 0..3 {
                let (i, j, o) = (t.verts[k], t.verts[(k + 1) % 3], p[(k + 2) % 3]);
                let (a, b) = (p[k] - o, p[(k + 1) % 3] - o);
                let s = a.cross(b).length();
                let cot = if s > 1e-12 { a.dot(b) / s } else { 0.0 };
                *w.entry(ekey(i, j)).or_insert(0.0) += (cot * 0.5).max(1e-4);
            }
        }
        for e in &m.loose_edges {
            w.entry(ekey(e[0], e[1])).or_insert(1.0);
        }
        let mut nbrs: Vec<Vec<(u32, f32)>> = vec![Vec::new(); nv];
        for (&(a, b), &x) in &w {
            nbrs[a as usize].push((b, x));
            nbrs[b as usize].push((a, x));
        }
        let x0 = m.positions.clone();
        for _ in 0..40 {
            for &v in &sel {
                let nb = &nbrs[v as usize];
                let tw: f32 = nb.iter().map(|n| n.1).sum();
                if nb.is_empty() || tw <= 0.0 {
                    continue;
                }
                let lam = if boundary.contains(&v) { lambda_border } else { lambda };
                let avg = nb.iter().fold(Vec3::ZERO, |a, &(u, x)| a + m.positions[u as usize] * (x / tw));
                m.positions[v as usize] = (x0[v as usize] + avg * lam) / (1.0 + lam);
            }
        }
    }
    if preserve_volume {
        let v1 = m.volume();
        if v0.abs() > 1e-12 && v1.abs() > 1e-12 && v0.signum() == v1.signum() {
            let s = (v0 / v1).cbrt();
            let c = sel.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / sel.len() as f32;
            for &v in &sel {
                let p = m.positions[v as usize];
                m.positions[v as usize] = c + (p - c) * s;
            }
        }
    }
}
