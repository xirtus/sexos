// SPDX-License-Identifier: GPL-3.0-only
//! Convex Hull.
use super::*;

type D3 = [f64; 3];
fn sub(a: D3, b: D3) -> D3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: D3, b: D3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: D3, b: D3) -> D3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn len(a: D3) -> f64 {
    dot(a, a).sqrt()
}

struct HFace {
    v: [usize; 3],
    n: D3,
    d: f64,
    alive: bool,
}

fn make(p: &[D3], a: usize, b: usize, c: usize) -> HFace {
    let n = cross(sub(p[b], p[a]), sub(p[c], p[a]));
    let l = len(n).max(1e-300);
    let n = [n[0] / l, n[1] / l, n[2] / l];
    HFace { v: [a, b, c], n, d: dot(n, p[a]), alive: true }
}

/// Outward, counter-clockwise triangles of the convex hull of `pts`
/// (incremental hull in f64). Points on the hull's faces are left out. None
/// when the points are fewer than four or all coplanar.
pub fn convex_hull_triangles(pts: &[Vec3]) -> Option<Vec<[usize; 3]>> {
    let n = pts.len();
    if n < 4 {
        return None;
    }
    let p: Vec<D3> = pts.iter().map(|v| [v.x as f64, v.y as f64, v.z as f64]).collect();
    let mut lo = p[0];
    let mut hi = p[0];
    for q in &p {
        for k in 0..3 {
            lo[k] = lo[k].min(q[k]);
            hi[k] = hi[k].max(q[k]);
        }
    }
    let eps = 1e-7 * len(sub(hi, lo)).max(1.0);
    let i0 = (0..n).min_by(|&a, &b| p[a][0].total_cmp(&p[b][0]).then(p[a][1].total_cmp(&p[b][1])).then(p[a][2].total_cmp(&p[b][2])))?;
    let i1 = (0..n).max_by(|&a, &b| len(sub(p[a], p[i0])).total_cmp(&len(sub(p[b], p[i0]))))?;
    if len(sub(p[i1], p[i0])) <= eps {
        return None;
    }
    let dir = sub(p[i1], p[i0]);
    let line_d = |q: D3| len(cross(dir, sub(q, p[i0]))) / len(dir);
    let i2 = (0..n).max_by(|&a, &b| line_d(p[a]).total_cmp(&line_d(p[b])))?;
    if line_d(p[i2]) <= eps {
        return None;
    }
    let base = make(&p, i0, i1, i2);
    let plane_d = |q: D3| dot(base.n, q) - base.d;
    let i3 = (0..n).max_by(|&a, &b| plane_d(p[a]).abs().total_cmp(&plane_d(p[b]).abs()))?;
    if plane_d(p[i3]).abs() <= eps {
        return None;
    }
    let inside = [0, 1, 2].map(|k| (p[i0][k] + p[i1][k] + p[i2][k] + p[i3][k]) / 4.0);
    let mut faces: Vec<HFace> = Vec::new();
    for [a, b, c] in [[i0, i1, i2], [i0, i3, i1], [i1, i3, i2], [i2, i3, i0]] {
        let mut f = make(&p, a, b, c);
        if dot(f.n, inside) - f.d > 0.0 {
            f = make(&p, a, c, b);
        }
        faces.push(f);
    }
    for (i, &q) in p.iter().enumerate() {
        if i == i0 || i == i1 || i == i2 || i == i3 {
            continue;
        }
        let visible: Vec<usize> = (0..faces.len()).filter(|&f| faces[f].alive && dot(faces[f].n, q) - faces[f].d > eps).collect();
        if visible.is_empty() {
            continue;
        }
        let mut es: HashSet<(usize, usize)> = HashSet::new();
        for &f in &visible {
            let v = faces[f].v;
            for k in 0..3 {
                es.insert((v[k], v[(k + 1) % 3]));
            }
        }
        let mut horizon: Vec<(usize, usize)> = Vec::new();
        for &f in &visible {
            let v = faces[f].v;
            for k in 0..3 {
                let e = (v[k], v[(k + 1) % 3]);
                if !es.contains(&(e.1, e.0)) {
                    horizon.push(e);
                }
            }
            faces[f].alive = false;
        }
        for (a, b) in horizon {
            faces.push(make(&p, a, b, i));
        }
    }
    Some(faces.iter().filter(|f| f.alive).map(|f| f.v).collect())
}

/// Convex Hull (Blender's defaults: delete unused, join triangles at 40
/// degrees): the hull of the selected vertices replaces the selected faces;
/// selected vertices inside the hull and selected wire edges are deleted, and
/// hull triangles are joined into quads where they meet the Tris to Quads
/// limits. Leaves the hull faces selected.
pub fn convex_hull(m: &mut Mesh, verts: &[u32], faces: &[u32]) -> Result<Selection, String> {
    let mut idx: Vec<u32> = verts.iter().copied().filter(|&v| (v as usize) < m.positions.len()).collect();
    idx.sort_unstable();
    idx.dedup();
    let pts: Vec<Vec3> = idx.iter().map(|&v| m.positions[v as usize]).collect();
    let tris = convex_hull_triangles(&pts).ok_or("Convex hull failed: the selection is flat or has fewer than 4 vertices")?;
    let on_hull: HashSet<u32> = tris.iter().flat_map(|t| t.iter().map(|&i| idx[i])).collect();
    let sel: HashSet<u32> = idx.iter().copied().collect();
    let dead: HashSet<u32> = idx.iter().copied().filter(|v| !on_hull.contains(v)).collect();
    let fsel: HashSet<u32> = faces.iter().copied().collect();
    let remove: Vec<bool> = (0..m.faces.len()).map(|f| fsel.contains(&(f as u32)) || m.faces[f].verts.iter().any(|v| dead.contains(v))).collect();
    retain_faces(m, &remove);
    m.loose_edges.retain(|e| !(sel.contains(&e[0]) && sel.contains(&e[1])) && !dead.contains(&e[0]) && !dead.contains(&e[1]));
    let base = m.faces.len() as u32;
    for t in &tris {
        m.faces.push(Face::new(t.iter().map(|&i| idx[i]).collect()));
    }
    let new: Vec<u32> = (base..m.faces.len() as u32).collect();
    let limit = 40f32.to_radians();
    let joined = super::fill::tris_to_quads(m, &new, limit, limit).faces;
    crate::ops::fill_missing_uvs(m);
    compact(m);
    Ok(Selection::faces(m, joined))
}
