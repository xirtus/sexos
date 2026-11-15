// SPDX-License-Identifier: GPL-3.0-only
//! Connect Vertex Pairs / Path, Split Concave and Split Non-Planar Faces.
use super::*;

/// Connect Vertex Pairs: in every face holding two or more selected
/// vertices, the selected corners are joined in face order by new edges
/// (two corners: one cut; more: each run between consecutive selected
/// corners is cut off, leaving a face of the selected corners in the
/// middle). Leaves the new edges selected.
pub fn vert_connect(m: &mut Mesh, verts: &[u32]) -> Selection {
    let sel: HashSet<u32> = verts.iter().copied().collect();
    let mut new_edges = Vec::new();
    let nf = m.faces.len();
    let mut existing: HashSet<(u32, u32)> = m.edges().into_iter().map(|e| (e[0], e[1])).collect();
    for fi in 0..nf {
        let f = m.faces[fi].clone();
        let n = f.verts.len();
        let s: Vec<usize> = (0..n).filter(|&i| sel.contains(&f.verts[i])).collect();
        if s.len() < 2 || n < 4 {
            continue;
        }
        let cs = corners(&f);
        let mut pieces: Vec<Vec<Corner>> = Vec::new();
        let mut center: Vec<Corner> = Vec::new();
        for (j, &a) in s.iter().enumerate() {
            let b = s[(j + 1) % s.len()];
            let gap = (b + n - a) % n;
            let (va, vb) = (f.verts[a], f.verts[b]);
            center.push(cs[a]);
            if gap >= 2 && !existing.contains(&ekey(va, vb)) && (s.len() > 2 || j == 0) {
                let piece: Vec<Corner> = (0..=gap).map(|k| cs[(a + k) % n]).collect();
                pieces.push(piece);
                new_edges.push(ek(va, vb));
                existing.insert(ekey(va, vb));
            } else {
                for k in 1..gap {
                    center.push(cs[(a + k) % n]);
                }
            }
        }
        if pieces.is_empty() {
            continue;
        }
        if s.len() == 2 {
            // Two corners: the cut and the rest of the face.
            let (a, b) = (s[0], s[1]);
            let gap = (n + a - b) % n;
            pieces.push((0..=gap).map(|k| cs[(b + k) % n]).collect());
        } else if center.len() >= 3 {
            pieces.push(center);
        }
        let mut it = pieces.into_iter();
        let first = it.next().unwrap_or_default();
        m.faces[fi] = face_from(&f, &first);
        for p in it {
            m.faces.push(face_from(&f, &p));
        }
    }
    Selection::edges(new_edges)
}

/// Connect two vertices with a path of new edges across the faces between
/// them, like Blender's connect_vert_pair: the path follows the plane through
/// both vertices that contains their mean normal, splitting the faces and
/// edges it crosses. Returns the new edges, or None (mesh unchanged) when
/// no path is found.
pub fn connect_vert_pair(m: &mut Mesh, a: u32, b: u32) -> Option<Vec<[u32; 2]>> {
    if a == b || a as usize >= m.positions.len() || b as usize >= m.positions.len() {
        return None;
    }
    // Same face: one split.
    if let Some(f) = (0..m.faces.len()).find(|&f| m.faces[f].verts.contains(&a) && m.faces[f].verts.contains(&b)) {
        if m.edges().contains(&ek(a, b)) {
            return Some(Vec::new());
        }
        split_face(m, f as u32, a, b)?;
        return Some(vec![ek(a, b)]);
    }
    let mut w = m.clone();
    let vf = vert_faces(&w);
    let (pa, pb) = (w.positions[a as usize], w.positions[b as usize]);
    let mut n = vertex_normal(&w, &vf, a) + vertex_normal(&w, &vf, b);
    let dir = pb - pa;
    if n.length() < 1e-6 {
        n = dir.normalize().any_orthonormal();
    }
    let mut pn = dir.cross(n).normalize();
    if pn.length() < 0.5 {
        pn = dir.normalize().any_orthonormal();
    }
    let eps = 1e-6 * scale_of(&w);
    let side = |p: Vec3| (p - pa).dot(pn);
    let prog = |p: Vec3| (p - pa).dot(dir);
    let mut edges = Vec::new();
    let mut cur = a;
    let mut visited: HashSet<u32> = HashSet::new();
    // Face the walk must continue in (after crossing an edge).
    let mut forced: Option<u32> = None;
    for _ in 0..w.faces.len() * 2 + 4 {
        let candidates: Vec<u32> = match forced {
            Some(f) => vec![f],
            None => (0..w.faces.len() as u32).filter(|&f| w.faces[f as usize].verts.contains(&cur) && !visited.contains(&f)).collect(),
        };
        let pc = prog(w.positions[cur as usize]);
        // Best crossing: (face, kind, progress) where kind is Ok(vertex) or Err(edge).
        let mut best: Option<(u32, Result<u32, (u32, u32, f32)>, f32)> = None;
        for &f in &candidates {
            let fv = w.faces[f as usize].verts.clone();
            if fv.contains(&b) {
                best = Some((f, Ok(b), f32::MAX));
                break;
            }
            let k = fv.len();
            let ic = fv.iter().position(|&x| x == cur).unwrap_or(0);
            for i in 0..k {
                let (p, q) = (fv[i], fv[(i + 1) % k]);
                if p == cur || q == cur {
                    continue;
                }
                let (dp, dq) = (side(w.positions[p as usize]), side(w.positions[q as usize]));
                let cand = if dp.abs() <= eps {
                    Some((Ok(p), w.positions[p as usize]))
                } else if dq.abs() <= eps {
                    Some((Ok(q), w.positions[q as usize]))
                } else if dp * dq < 0.0 {
                    let t = dp / (dp - dq);
                    Some((Err((p, q, t)), w.positions[p as usize].lerp(w.positions[q as usize], t)))
                } else {
                    None
                };
                if let Some((kind, x)) = cand {
                    // A neighbouring corner on the plane means walking along an edge.
                    if let Ok(v) = kind {
                        if v == fv[(ic + 1) % k] || v == fv[(ic + k - 1) % k] {
                            continue;
                        }
                    }
                    let g = prog(x);
                    if g > pc + eps && g <= prog(pb) + eps && best.as_ref().map_or(true, |bb| g > bb.2 && bb.2 != f32::MAX) {
                        best = Some((f, kind, g));
                    }
                }
            }
        }
        let (f, kind, _) = best?;
        visited.insert(f);
        match kind {
            Ok(v) => {
                if !w.faces[f as usize].edges().any(|e| ekey(e.0, e.1) == ekey(cur, v)) {
                    split_face(&mut w, f, cur, v)?;
                    edges.push(ek(cur, v));
                }
                if v == b {
                    *m = w;
                    return Some(edges);
                }
                cur = v;
                forced = None;
            }
            Err((p, q, t)) => {
                let x = w.add_vertex(w.positions[p as usize].lerp(w.positions[q as usize], t));
                // The face on the other side of p-q, before inserting.
                let other = (0..w.faces.len() as u32).find(|&g| g != f && w.faces[g as usize].edges().any(|e| ekey(e.0, e.1) == ekey(p, q)));
                insert_edge_vertex(&mut w, p, q, x, t);
                split_face(&mut w, f, cur, x)?;
                edges.push(ek(cur, x));
                cur = x;
                match other {
                    Some(g) if !visited.contains(&g) => forced = Some(g),
                    _ => return None,
                }
            }
        }
    }
    None
}

/// Connect Vertex Path: join the vertices in the given order (Blender's
/// selection order) with [`connect_vert_pair`]. Leaves the new edges and the
/// path's vertices selected.
pub fn vert_connect_path(m: &mut Mesh, order: &[u32]) -> Selection {
    let mut edges = Vec::new();
    for w in order.windows(2) {
        if let Some(es) = connect_vert_pair(m, w[0], w[1]) {
            edges.extend(es);
        }
    }
    let mut s = Selection::edges(edges);
    s.verts.extend(order.iter().copied());
    s.verts.sort_unstable();
    s.verts.dedup();
    s
}

/// Is the polygon (in its own plane) convex? Collinear corners are allowed.
fn convex(m: &Mesh, cs: &[Corner], n: Vec3) -> bool {
    let k = cs.len();
    (0..k).all(|i| {
        let (a, b, c) = (m.positions[cs[(i + k - 1) % k].0 as usize], m.positions[cs[i].0 as usize], m.positions[cs[(i + 1) % k].0 as usize]);
        (b - a).cross(c - b).dot(n) >= -1e-7 * (b - a).length() * (c - b).length()
    })
}

/// Split Concave Faces: each selected concave face is triangulated and the
/// triangles are merged back into the largest convex pieces.
pub fn vert_connect_concave(m: &mut Mesh, faces: &[u32]) -> Selection {
    let mut out = Vec::new();
    for &f in faces {
        let fi = f as usize;
        if fi >= m.faces.len() || m.faces[fi].verts.len() < 4 {
            out.push(f);
            continue;
        }
        let n = m.face_normal(fi);
        let src = m.faces[fi].clone();
        let cs = corners(&src);
        if convex(m, &cs, n) {
            out.push(f);
            continue;
        }
        let tmp = Mesh { positions: m.positions.clone(), faces: vec![src.clone()], loose_edges: Vec::new() };
        let mut polys: Vec<Vec<Corner>> = tmp.triangulate().iter().map(|t| t.corners.iter().map(|&c| cs[c as usize]).collect()).collect();
        // Merge across internal edges while the union stays convex, longest edges first.
        loop {
            let mut best: Option<(usize, usize, Vec<Corner>, f32)> = None;
            for i in 0..polys.len() {
                for j in i + 1..polys.len() {
                    if let Some((merged, len)) = merge_polys(m, &polys[i], &polys[j]) {
                        if convex(m, &merged, n) && best.as_ref().map_or(true, |b| len > b.3) {
                            best = Some((i, j, merged, len));
                        }
                    }
                }
            }
            let Some((i, j, merged, _)) = best else { break };
            polys[i] = merged;
            polys.remove(j);
        }
        let mut it = polys.into_iter();
        m.faces[fi] = face_from(&src, &it.next().unwrap_or_default());
        out.push(f);
        for p in it {
            m.faces.push(face_from(&src, &p));
            out.push(m.faces.len() as u32 - 1);
        }
    }
    Selection::faces(m, out)
}

/// Join two polygons sharing exactly one edge; returns the union and the
/// shared edge's length.
fn merge_polys(m: &Mesh, a: &[Corner], b: &[Corner]) -> Option<(Vec<Corner>, f32)> {
    let (na, nb) = (a.len(), b.len());
    for i in 0..na {
        let (x, y) = (a[i].0, a[(i + 1) % na].0);
        if let Some(j) = (0..nb).find(|&j| b[j].0 == y && b[(j + 1) % nb].0 == x) {
            // a: ... x y ...; b: ... y x ...  -> a up to x, then b after x around to before y...
            let mut out = Vec::with_capacity(na + nb - 2);
            for k in 0..na {
                out.push(a[(i + 1 + k) % na]); // starts at y, ends at x
            }
            for k in 2..nb {
                out.push(b[(j + k) % nb]); // after x, up to before y
            }
            let mut seen = HashSet::new();
            if !out.iter().all(|c| seen.insert(c.0)) {
                return None;
            }
            let len = m.positions[x as usize].distance(m.positions[y as usize]);
            return Some((out, len));
        }
    }
    None
}

/// Largest angle between a polygon's normal and the normals of its corner
/// triangles: zero for a planar polygon.
fn nonplanarity(m: &Mesh, cs: &[Corner]) -> f32 {
    let k = cs.len();
    if k < 4 {
        return 0.0;
    }
    let p = |i: usize| m.positions[cs[i % k].0 as usize];
    let mut n = Vec3::ZERO;
    for i in 0..k {
        let (a, b) = (p(i), p(i + 1));
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    let n = n.normalize();
    let mut worst = 0.0f32;
    for i in 0..k {
        let t = (p(i) - p(i + k - 1)).cross(p(i + 1) - p(i)).normalize();
        if t.length() > 0.5 {
            worst = worst.max(t.dot(n).clamp(-1.0, 1.0).acos());
        }
    }
    worst
}

/// Split Non-Planar Faces: faces bending more than `angle_limit` (Blender's
/// default is 5 degrees) are split recursively along the diagonal that
/// leaves the flattest pieces.
pub fn vert_connect_nonplanar(m: &mut Mesh, faces: &[u32], angle_limit: f32) -> Selection {
    let mut out = Vec::new();
    for &f in faces {
        let fi = f as usize;
        if fi >= m.faces.len() {
            continue;
        }
        let src = m.faces[fi].clone();
        let mut done = Vec::new();
        let mut todo = vec![corners(&src)];
        while let Some(p) = todo.pop() {
            if p.len() < 4 || nonplanarity(m, &p) <= angle_limit {
                done.push(p);
                continue;
            }
            let k = p.len();
            let mut best: Option<(f32, usize, usize)> = None;
            for i in 0..k {
                for j in i + 2..k {
                    if i == 0 && j == k - 1 {
                        continue;
                    }
                    let a: Vec<Corner> = p[i..=j].to_vec();
                    let b: Vec<Corner> = p[j..].iter().chain(p[..=i].iter()).copied().collect();
                    let cost = nonplanarity(m, &a).max(nonplanarity(m, &b));
                    if best.map_or(true, |bb| cost < bb.0 - 1e-6) {
                        best = Some((cost, i, j));
                    }
                }
            }
            match best {
                Some((_, i, j)) => {
                    todo.push(p[i..=j].to_vec());
                    todo.push(p[j..].iter().chain(p[..=i].iter()).copied().collect());
                }
                None => done.push(p),
            }
        }
        let mut it = done.into_iter();
        m.faces[fi] = face_from(&src, &it.next().unwrap_or_default());
        out.push(f);
        for p in it {
            m.faces.push(face_from(&src, &p));
            out.push(m.faces.len() as u32 - 1);
        }
    }
    Selection::faces(m, out)
}
