// SPDX-License-Identifier: GPL-3.0-only
//! Tris to Quads, Beautify Faces, Fill, Grid Fill, Weld Edges into Faces.
use super::*;

/// The corner of triangle `f` opposite its edge (a, b), if `f` is a triangle.
fn third(m: &Mesh, f: u32, a: u32, b: u32) -> Option<u32> {
    let v = &m.faces[f as usize].verts;
    (v.len() == 3).then(|| v.iter().copied().find(|&x| x != a && x != b)).flatten()
}

fn uv_lookup(f: &Face) -> HashMap<u32, Vec2> {
    corners(f).into_iter().collect()
}

/// Angle at `b` between b->a and b->c.
fn corner_angle(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (u, v) = ((a - b).normalize(), (c - b).normalize());
    u.dot(v).clamp(-1.0, 1.0).acos()
}

/// Is the quad (in order) convex with respect to normal `n`?
fn quad_convex(p: [Vec3; 4], n: Vec3) -> bool {
    (0..4).all(|i| (p[i] - p[(i + 3) % 4]).cross(p[(i + 1) % 4] - p[i]).dot(n) > 0.0)
}

/// Tris to Quads (Alt+J): join pairs of selected triangles across shared
/// edges into quads, best pairs first, when the two normals differ by at most
/// `face_threshold` and the quad's corners stay within `shape_threshold` of
/// right angles (Blender's defaults are 40 degrees for both). Leaves the
/// quads and the unjoined triangles selected.
pub fn tris_to_quads(m: &mut Mesh, faces: &[u32], face_threshold: f32, shape_threshold: f32) -> Selection {
    let sel: HashSet<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len() && m.faces[f as usize].verts.len() == 3).collect();
    let dir = dir_map(m);
    let mut cands: Vec<(f32, u32, u32, [u32; 4])> = Vec::new();
    let mut sorted: Vec<u32> = sel.iter().copied().collect();
    sorted.sort_unstable();
    for &f in &sorted {
        for (a, b) in m.faces[f as usize].edges() {
            let Some(&g) = dir.get(&(b, a)) else { continue };
            if g <= f || !sel.contains(&g) {
                continue;
            }
            let (Some(x), Some(y)) = (third(m, f, a, b), third(m, g, b, a)) else { continue };
            if x == y {
                continue;
            }
            let (nf, ng) = (m.face_normal(f as usize), m.face_normal(g as usize));
            let ang = nf.dot(ng).clamp(-1.0, 1.0).acos();
            if ang > face_threshold {
                continue;
            }
            let q = [a, y, b, x];
            let p = q.map(|v| m.positions[v as usize]);
            if !quad_convex(p, (nf + ng).normalize()) {
                continue;
            }
            let dev = (0..4).map(|i| (corner_angle(p[(i + 3) % 4], p[i], p[(i + 1) % 4]) - core::f32::consts::FRAC_PI_2).abs()).fold(0.0f32, f32::max);
            if dev > shape_threshold {
                continue;
            }
            let score = ang / face_threshold.max(1e-6) + dev / shape_threshold.max(1e-6);
            cands.push((score, f, g, q));
        }
    }
    cands.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut used: HashSet<u32> = HashSet::new();
    let mut remove = vec![false; m.faces.len()];
    let mut joined = Vec::new();
    for (_, f, g, q) in cands {
        if used.contains(&f) || used.contains(&g) {
            continue;
        }
        used.insert(f);
        used.insert(g);
        let (uf, ug) = (uv_lookup(&m.faces[f as usize]), uv_lookup(&m.faces[g as usize]));
        let cs: Vec<Corner> = q.iter().map(|&v| (v, uf.get(&v).or_else(|| ug.get(&v)).copied().unwrap_or(Vec2::ZERO))).collect();
        let nf = face_from(&m.faces[f as usize], &cs);
        m.faces[f as usize] = nf;
        remove[g as usize] = true;
        joined.push(f);
    }
    let map = retain_faces(m, &remove);
    let out: Vec<u32> = sorted.iter().map(|&f| map[f as usize]).filter(|&f| f != NONE).collect();
    Selection::faces(m, out)
}

/// Beautify Faces: flip the shared edges of selected triangles until every
/// pair meets the Delaunay condition (the angles opposite a shared edge sum
/// to at most 180 degrees), which removes long thin triangles. Face indices
/// are unchanged.
pub fn beautify_fill(m: &mut Mesh, faces: &[u32]) -> Selection {
    let sel: Vec<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len() && m.faces[f as usize].verts.len() == 3).collect();
    let selset: HashSet<u32> = sel.iter().copied().collect();
    let mut dir = dir_map(m);
    let mut edges: HashSet<(u32, u32)> = m.edges().into_iter().map(|e| (e[0], e[1])).collect();
    for _pass in 0..200 {
        let mut changed = false;
        for &f in &sel {
            let fe: Vec<(u32, u32)> = m.faces[f as usize].edges().collect();
            for (a, b) in fe {
                let Some(&g) = dir.get(&(b, a)) else { continue };
                if g == f || !selset.contains(&g) || m.faces[f as usize].verts.len() != 3 {
                    continue;
                }
                // The face may have changed under us this pass.
                if !m.faces[f as usize].edges().any(|e| e == (a, b)) {
                    break;
                }
                let (Some(x), Some(y)) = (third(m, f, a, b), third(m, g, b, a)) else { continue };
                if x == y || edges.contains(&ekey(x, y)) {
                    continue;
                }
                let [pa, pb, px, py] = [a, b, x, y].map(|v| m.positions[v as usize]);
                let n = (m.face_normal(f as usize) + m.face_normal(g as usize)).normalize();
                if !quad_convex([pa, py, pb, px], n) {
                    continue;
                }
                if corner_angle(pa, px, pb) + corner_angle(pa, py, pb) <= core::f32::consts::PI + 1e-4 {
                    continue;
                }
                let (uf, ug) = (uv_lookup(&m.faces[f as usize]), uv_lookup(&m.faces[g as usize]));
                let uv = |v: u32| uf.get(&v).or_else(|| ug.get(&v)).copied().unwrap_or(Vec2::ZERO);
                for e in m.faces[f as usize].edges().chain(m.faces[g as usize].edges()).collect::<Vec<_>>() {
                    dir.remove(&e);
                }
                let nf = face_from(&m.faces[f as usize], &[(x, uv(x)), (a, uv(a)), (y, uv(y))]);
                let ng = face_from(&m.faces[g as usize], &[(y, uv(y)), (b, uv(b)), (x, uv(x))]);
                m.faces[f as usize] = nf;
                m.faces[g as usize] = ng;
                for e in m.faces[f as usize].edges().collect::<Vec<_>>() {
                    dir.insert(e, f);
                }
                for e in m.faces[g as usize].edges().collect::<Vec<_>>() {
                    dir.insert(e, g);
                }
                edges.remove(&ekey(a, b));
                edges.insert(ekey(x, y));
                changed = true;
                break;
            }
        }
        if !changed {
            break;
        }
    }
    Selection::faces(m, sel)
}

/// Closed loops made of the selected edges (every vertex on exactly two of
/// them), each ordered so that a face along it winds against the faces
/// already using its edges.
fn selected_loops(m: &Mesh, edges: &[[u32; 2]]) -> Vec<Vec<u32>> {
    let mut nb: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut set = HashSet::new();
    for e in edges {
        if e[0] != e[1] && set.insert(ekey(e[0], e[1])) {
            nb.entry(e[0]).or_default().push(e[1]);
            nb.entry(e[1]).or_default().push(e[0]);
        }
    }
    let dir = dir_map(m);
    let mut seen: HashSet<u32> = HashSet::new();
    let mut keys: Vec<u32> = nb.keys().copied().collect();
    keys.sort_unstable();
    let mut loops = Vec::new();
    for s in keys {
        if seen.contains(&s) {
            continue;
        }
        // Collect the component.
        let mut comp = vec![s];
        seen.insert(s);
        let mut i = 0;
        while i < comp.len() {
            for &u in &nb[&comp[i]] {
                if seen.insert(u) {
                    comp.push(u);
                }
            }
            i += 1;
        }
        if comp.len() < 3 || comp.iter().any(|v| nb[v].len() != 2) {
            continue;
        }
        let mut lp = vec![s];
        let mut prev = s;
        let mut cur = nb[&s][0];
        while cur != s && lp.len() <= comp.len() {
            lp.push(cur);
            let nx = if nb[&cur][0] == prev { nb[&cur][1] } else { nb[&cur][0] };
            prev = cur;
            cur = nx;
        }
        if lp.len() != comp.len() {
            continue;
        }
        let k = lp.len();
        if (0..k).any(|i| dir.contains_key(&(lp[i], lp[(i + 1) % k]))) {
            lp.reverse();
        }
        loops.push(lp);
    }
    loops
}

/// Fill (Alt+F): triangulate each closed loop of selected edges (beautified
/// like Blender's default). Leaves the new triangles selected.
pub fn fill(m: &mut Mesh, edges: &[[u32; 2]], beauty: bool) -> Selection {
    let loops = selected_loops(m, edges);
    let mut new_faces = Vec::new();
    for lp in loops {
        let tmp = Mesh { positions: m.positions.clone(), faces: vec![Face::new(lp.clone())], loose_edges: Vec::new() };
        for t in tmp.triangulate() {
            m.faces.push(Face::new(t.verts.to_vec()));
            new_faces.push(m.faces.len() as u32 - 1);
        }
        let k = lp.len();
        let es: HashSet<(u32, u32)> = (0..k).map(|i| ekey(lp[i], lp[(i + 1) % k])).collect();
        m.loose_edges.retain(|e| !es.contains(&ekey(e[0], e[1])));
    }
    if beauty {
        beautify_fill(m, &new_faces);
    }
    crate::ops::fill_missing_uvs(m);
    Selection::faces(m, new_faces)
}

/// Grid Fill: fill a closed loop of selected edges (an even number of them,
/// at least four) with a grid of quads. The loop's four corners are its
/// four sharpest turns when they give opposite sides of equal length;
/// otherwise the sides are a quarter of the loop (Blender's span) starting at
/// its sharpest corner. Inner points follow a Coons patch of the sides.
pub fn fill_grid(m: &mut Mesh, edges: &[[u32; 2]]) -> Result<Selection, String> {
    let loops = selected_loops(m, edges);
    if loops.is_empty() {
        return Err("Select a closed loop of edges".into());
    }
    let mut new_faces = Vec::new();
    for lp in loops {
        let n = lp.len();
        if n % 2 != 0 || n < 4 {
            return Err(format!("Grid fill needs an even number of edges (got {n})"));
        }
        let p = |i: usize| m.positions[lp[i % n] as usize];
        let angles: Vec<f32> = (0..n).map(|i| corner_angle(p(i + n - 1), p(i), p(i + 1))).collect();
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| angles[a].total_cmp(&angles[b]).then(a.cmp(&b)));
        let mut corners4: Vec<usize> = order[..4].to_vec();
        corners4.sort_unstable();
        let sides: Vec<usize> = (0..4).map(|i| (corners4[(i + 1) % 4] + n - corners4[i]) % n).collect();
        let (start, a, b) = if sides[0] == sides[2] && sides[1] == sides[3] && sides.iter().all(|&s| s >= 1) {
            (corners4[0], sides[0], sides[1])
        } else {
            let a = (n / 4).max(1);
            (order[0], a, n / 2 - a)
        };
        let at = |i: usize| lp[(start + i) % n];
        let bottom: Vec<u32> = (0..=a).map(at).collect();
        let right: Vec<u32> = (0..=b).map(|j| at(a + j)).collect();
        let top: Vec<u32> = (0..=a).map(|i| at(2 * a + b - i)).collect();
        let left: Vec<u32> = (0..=b).map(|j| at(n - j)).collect();
        let snapshot = m.positions.clone();
        let pos = |v: u32| snapshot[v as usize];
        let (c00, c10, c11, c01) = (pos(bottom[0]), pos(bottom[a]), pos(top[a]), pos(top[0]));
        let mut g = vec![vec![0u32; b + 1]; a + 1];
        for i in 0..=a {
            g[i][0] = bottom[i];
            g[i][b] = top[i];
        }
        for j in 0..=b {
            g[0][j] = left[j];
            g[a][j] = right[j];
        }
        for i in 1..a {
            for j in 1..b {
                let (u, v) = (i as f32 / a as f32, j as f32 / b as f32);
                let q = pos(bottom[i]) * (1.0 - v) + pos(top[i]) * v + pos(left[j]) * (1.0 - u) + pos(right[j]) * u
                    - (c00 * ((1.0 - u) * (1.0 - v)) + c10 * (u * (1.0 - v)) + c01 * ((1.0 - u) * v) + c11 * (u * v));
                g[i][j] = m.add_vertex(q);
            }
        }
        for j in 0..b {
            for i in 0..a {
                m.faces.push(Face::new(vec![g[i][j], g[i + 1][j], g[i + 1][j + 1], g[i][j + 1]]));
                new_faces.push(m.faces.len() as u32 - 1);
            }
        }
        let es: HashSet<(u32, u32)> = (0..n).map(|i| ekey(lp[i], lp[(i + 1) % n])).collect();
        m.loose_edges.retain(|e| !es.contains(&ekey(e[0], e[1])));
    }
    crate::ops::fill_missing_uvs(m);
    Ok(Selection::faces(m, new_faces))
}

/// Weld Edges into Faces: selected wire edges (and chains of them through
/// wire-only vertices) that join two corners of a face split that face.
/// Returns the welded edges selected.
pub fn face_split_by_edges(m: &mut Mesh, edges: &[[u32; 2]]) -> Selection {
    let want: HashSet<(u32, u32)> = edges.iter().map(|e| ekey(e[0], e[1])).collect();
    let wires: Vec<(u32, u32)> = m.loose_edges.iter().map(|e| ekey(e[0], e[1])).filter(|k| want.contains(k)).collect();
    let in_face: HashSet<u32> = m.faces.iter().flat_map(|f| f.verts.iter().copied()).collect();
    let mut nb: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(a, b) in &wires {
        nb.entry(a).or_default().push(b);
        nb.entry(b).or_default().push(a);
    }
    let mut used: HashSet<(u32, u32)> = HashSet::new();
    let mut chains: Vec<Vec<u32>> = Vec::new();
    let mut starts: Vec<u32> = nb.keys().copied().filter(|v| in_face.contains(v)).collect();
    starts.sort_unstable();
    for s in starts {
        for &first in &nb[&s].clone() {
            if used.contains(&ekey(s, first)) {
                continue;
            }
            let mut chain = vec![s, first];
            let mut prev = s;
            let mut cur = first;
            while !in_face.contains(&cur) && nb[&cur].len() == 2 && chain.len() <= wires.len() + 1 {
                let nx = if nb[&cur][0] == prev { nb[&cur][1] } else { nb[&cur][0] };
                chain.push(nx);
                prev = cur;
                cur = nx;
            }
            if in_face.contains(&cur) && cur != s {
                for w in chain.windows(2) {
                    used.insert(ekey(w[0], w[1]));
                }
                chains.push(chain);
            }
        }
    }
    let mut welded = Vec::new();
    for chain in chains {
        let (a, b) = (chain[0], *chain.last().unwrap());
        let inner: Vec<u32> = chain[1..chain.len() - 1].to_vec();
        let Some(f) = (0..m.faces.len()).find(|&f| m.faces[f].verts.contains(&a) && m.faces[f].verts.contains(&b)) else { continue };
        let face = m.faces[f].clone();
        let n = face.verts.len();
        let ia = face.verts.iter().position(|&v| v == a).unwrap();
        let ib = face.verts.iter().position(|&v| v == b).unwrap();
        if inner.is_empty() && ((ia + 1) % n == ib || (ib + 1) % n == ia) {
            continue;
        }
        let cs = corners(&face);
        let (ua, ub) = (cs[ia].1, cs[ib].1);
        let k = inner.len() + 1;
        let inner_c: Vec<Corner> = inner.iter().enumerate().map(|(i, &v)| (v, ua.lerp(ub, (i + 1) as f32 / k as f32))).collect();
        let arc = |from: usize, to: usize| -> Vec<Corner> {
            let mut out = Vec::new();
            let mut i = from;
            loop {
                out.push(cs[i]);
                if i == to {
                    break;
                }
                i = (i + 1) % n;
            }
            out
        };
        let mut p1 = arc(ia, ib);
        p1.extend(inner_c.iter().rev().copied());
        let mut p2 = arc(ib, ia);
        p2.extend(inner_c.iter().copied());
        m.faces[f] = face_from(&face, &p1);
        m.faces.push(face_from(&face, &p2));
        let es: HashSet<(u32, u32)> = chain.windows(2).map(|w| ekey(w[0], w[1])).collect();
        m.loose_edges.retain(|e| !es.contains(&ekey(e[0], e[1])));
        welded.extend(chain.windows(2).map(|w| ek(w[0], w[1])));
    }
    Selection::edges(welded)
}
