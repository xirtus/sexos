// SPDX-License-Identifier: GPL-3.0-only
//! Subdivide (Blender's bmesh "subdivide_edges" patterns with the default
//! Straight Cut corner type) and Subdivide Edge-Ring.
use super::*;
use std::collections::BTreeSet;

/// Subdivide the selected edges with `cuts` evenly spaced vertices
/// (smoothness 0). Faces are split by Blender's patterns: a quad with all
/// four edges selected becomes a (cuts+1)^2 grid, a triangle with all three a
/// triangle grid, a triangle with one edge a fan to the opposite corner, any
/// face with exactly two selected edges is cut straight between them (and a
/// quad with three uses its opposite pair); other faces just gain the new
/// vertices along their edges. Leaves the subdivided region selected.
pub fn subdivide(m: &mut Mesh, edges: &[[u32; 2]], cuts: u32) -> Selection {
    subdivide_core(m, edges, cuts, false, &|m: &Mesh, a: u32, b: u32, t: f32| m.positions[a as usize].lerp(m.positions[b as usize], t))
}

/// Subdivide Edge-Ring: every quad crossed by the selected ring (two
/// opposite selected edges) is cut into `cuts` + 1 strips. New vertices
/// follow a Hermite curve whose end tangents are the edge turned into the
/// tangent planes of its vertices, blended in by `smoothness` (Blender's
/// defaults are 10 cuts and smoothness 1), so flat and cylindrical surfaces
/// stay straight and curved ones bulge with them.
pub fn subdivide_edgering(m: &mut Mesh, edges: &[[u32; 2]], cuts: u32, smoothness: f32) -> Selection {
    let vf = vert_faces(m);
    let normals: Vec<Vec3> = (0..m.positions.len() as u32).map(|v| vertex_normal(m, &vf, v)).collect();
    let pos = move |m: &Mesh, a: u32, b: u32, t: f32| -> Vec3 {
        let (pa, pb) = (m.positions[a as usize], m.positions[b as usize]);
        let d = pb - pa;
        let len = d.length();
        let tangent = |n: Vec3| -> Vec3 {
            let p = d - n * n.dot(d);
            if p.length() < 1e-6 * len.max(1e-12) {
                d
            } else {
                p.normalize() * len
            }
        };
        let ta = d.lerp(tangent(normals[a as usize]), smoothness);
        let tb = d.lerp(tangent(normals[b as usize]), smoothness);
        let (t2, t3) = (t * t, t * t * t);
        pa * (2.0 * t3 - 3.0 * t2 + 1.0) + ta * (t3 - 2.0 * t2 + t) + pb * (-2.0 * t3 + 3.0 * t2) + tb * (t3 - t2)
    };
    subdivide_core(m, edges, cuts, true, &pos)
}

type PosFn<'a> = dyn Fn(&Mesh, u32, u32, f32) -> Vec3 + 'a;

fn arc(l: &[Corner], from: usize, to: usize) -> Vec<Corner> {
    let n = l.len();
    let mut out = Vec::new();
    let mut i = from;
    loop {
        out.push(l[i]);
        if i == to || out.len() > n {
            break;
        }
        i = (i + 1) % n;
    }
    out
}

/// Cut a polygon between the points of two of its edges: P[i] joins
/// Q[k-1-i] (both index lists into `l` in loop order, P first).
fn nested(l: &[Corner], p: &[usize], q: &[usize]) -> Vec<Vec<Corner>> {
    let k = p.len().min(q.len());
    let mut out = vec![arc(l, q[k - 1], p[0])];
    for i in 0..k - 1 {
        out.push(vec![l[p[i]], l[p[i + 1]], l[q[k - 2 - i]], l[q[k - 1 - i]]]);
    }
    out.push(arc(l, p[k - 1], q[0]));
    out
}

fn subdivide_core(m: &mut Mesh, edges: &[[u32; 2]], cuts: u32, ring_only: bool, pos: &PosFn) -> Selection {
    let cuts = cuts.max(1);
    let existing: HashSet<(u32, u32)> = m.edges().into_iter().map(|e| (e[0], e[1])).collect();
    let sel: BTreeSet<(u32, u32)> = edges.iter().map(|e| ekey(e[0], e[1])).filter(|k| existing.contains(k)).collect();
    if sel.is_empty() {
        return Selection::default();
    }
    let first_new = m.positions.len() as u32;
    let mut points: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for &(lo, hi) in &sel {
        let list: Vec<u32> = (1..=cuts)
            .map(|i| {
                let p = pos(m, lo, hi, i as f32 / (cuts + 1) as f32);
                m.add_vertex(p)
            })
            .collect();
        points.insert((lo, hi), list);
    }
    let along = |u: u32, v: u32| -> Option<Vec<u32>> {
        let l = points.get(&ekey(u, v))?;
        Some(if u < v { l.clone() } else { l.iter().rev().copied().collect() })
    };
    let n_cut = cuts as usize + 1;
    let old = std::mem::take(&mut m.faces);
    let mut extra = Vec::new();
    let mut faces = Vec::with_capacity(old.len());
    for f in old {
        let n = f.verts.len();
        let cs = corners(&f);
        let sel_e: Vec<usize> = (0..n).filter(|&i| points.contains_key(&ekey(f.verts[i], f.verts[(i + 1) % n]))).collect();
        if sel_e.is_empty() {
            faces.push(f);
            continue;
        }
        let mut l: Vec<Corner> = Vec::new();
        let mut corner_at = vec![0usize; n];
        let mut edge_pts: Vec<Vec<usize>> = vec![Vec::new(); n];
        for i in 0..n {
            corner_at[i] = l.len();
            l.push(cs[i]);
            if let Some(pts) = along(f.verts[i], f.verts[(i + 1) % n]) {
                for (j, &x) in pts.iter().enumerate() {
                    let t = (j + 1) as f32 / n_cut as f32;
                    l.push((x, cs[i].1.lerp(cs[(i + 1) % n].1, t)));
                    edge_pts[i].push(l.len() - 1);
                }
            }
        }
        // Corner, points, next corner along edge e.
        let seq = |e: usize| -> Vec<Corner> {
            let mut s = vec![l[corner_at[e]]];
            s.extend(edge_pts[e].iter().map(|&i| l[i]));
            s.push(l[corner_at[(e + 1) % n]]);
            s
        };
        let k = sel_e.len();
        let opposite = n == 4 && k == 2 && sel_e[1] - sel_e[0] == 2;
        let pieces: Vec<Vec<Corner>> = if !ring_only && n == 4 && k == 4 {
            quad_grid(m, &seq(0), &seq(1), &seq(2), &seq(3), n_cut)
        } else if !ring_only && n == 3 && k == 3 {
            tri_grid(m, &seq(0), &seq(1), &seq(2), n_cut)
        } else if !ring_only && n == 3 && k == 1 {
            let e = sel_e[0];
            let s = seq(e);
            let opp = cs[(e + 2) % 3];
            (0..s.len() - 1).map(|j| vec![s[j], s[j + 1], opp]).collect()
        } else if (k == 2 && !ring_only) || opposite {
            nested(&l, &edge_pts[sel_e[0]], &edge_pts[sel_e[1]])
        } else if !ring_only && n == 4 && k == 3 {
            let pair = if sel_e.contains(&0) && sel_e.contains(&2) { (0, 2) } else { (1, 3) };
            nested(&l, &edge_pts[pair.0], &edge_pts[pair.1])
        } else {
            vec![l.clone()]
        };
        let mut it = pieces.into_iter().filter(|p| p.len() >= 3);
        match it.next() {
            Some(first) => faces.push(face_from(&f, &first)),
            None => faces.push(face_from(&f, &l)),
        }
        for p in it {
            extra.push(face_from(&f, &p));
        }
    }
    faces.extend(extra);
    m.faces = faces;
    // Selected wire edges become chains.
    let loose = std::mem::take(&mut m.loose_edges);
    for e in loose {
        match along(e[0], e[1]) {
            Some(pts) => {
                let mut chain = vec![e[0]];
                chain.extend(pts);
                chain.push(e[1]);
                for w in chain.windows(2) {
                    m.loose_edges.push([w[0], w[1]]);
                }
            }
            None => m.loose_edges.push(e),
        }
    }
    let mut verts: Vec<u32> = (first_new..m.positions.len() as u32).collect();
    verts.extend(sel.iter().flat_map(|&(a, b)| [a, b]));
    Selection::verts(verts)
}

/// Grid of a quad from its four edge sequences (each corner to next corner,
/// `n` segments): Coons patch for the interior points.
fn quad_grid(m: &mut Mesh, e0: &[Corner], e1: &[Corner], e2: &[Corner], e3: &[Corner], n: usize) -> Vec<Vec<Corner>> {
    let b: Vec<Corner> = e0.to_vec();
    let r: Vec<Corner> = e1.to_vec();
    let t: Vec<Corner> = e2.iter().rev().copied().collect();
    let lf: Vec<Corner> = e3.iter().rev().copied().collect();
    let p = |m: &Mesh, c: &Corner| m.positions[c.0 as usize];
    let mut g = vec![vec![(0u32, Vec2::ZERO); n + 1]; n + 1];
    for i in 0..=n {
        g[i][0] = b[i];
        g[i][n] = t[i];
    }
    for j in 0..=n {
        g[0][j] = lf[j];
        g[n][j] = r[j];
    }
    let (c00, c10, c11, c01) = (b[0], b[n], t[n], t[0]);
    for i in 1..n {
        for j in 1..n {
            let (u, v) = (i as f32 / n as f32, j as f32 / n as f32);
            let pos = p(m, &b[i]) * (1.0 - v) + p(m, &t[i]) * v + p(m, &lf[j]) * (1.0 - u) + p(m, &r[j]) * u
                - (p(m, &c00) * ((1.0 - u) * (1.0 - v)) + p(m, &c10) * (u * (1.0 - v)) + p(m, &c01) * ((1.0 - u) * v) + p(m, &c11) * (u * v));
            let uv = b[i].1 * (1.0 - v) + t[i].1 * v + lf[j].1 * (1.0 - u) + r[j].1 * u - (c00.1 * ((1.0 - u) * (1.0 - v)) + c10.1 * (u * (1.0 - v)) + c01.1 * ((1.0 - u) * v) + c11.1 * (u * v));
            g[i][j] = (m.add_vertex(pos), uv);
        }
    }
    let mut out = Vec::with_capacity(n * n);
    for j in 0..n {
        for i in 0..n {
            out.push(vec![g[i][j], g[i + 1][j], g[i + 1][j + 1], g[i][j + 1]]);
        }
    }
    out
}

/// Triangle grid from the three edge sequences (`n` segments each).
fn tri_grid(m: &mut Mesh, e0: &[Corner], e1: &[Corner], e2: &[Corner], n: usize) -> Vec<Vec<Corner>> {
    let (c0, c1, c2) = (e0[0], e1[0], e2[0]);
    let (p0, p1, p2) = (m.positions[c0.0 as usize], m.positions[c1.0 as usize], m.positions[c2.0 as usize]);
    let e2r: Vec<Corner> = e2.iter().rev().copied().collect();
    let mut t: Vec<Vec<Corner>> = Vec::with_capacity(n + 1);
    for r in 0..=n {
        let mut row = Vec::with_capacity(r + 1);
        for s in 0..=r {
            let c = if r == n {
                e1[s]
            } else if s == 0 {
                e0[r]
            } else if s == r {
                e2r[r]
            } else {
                let (a, b) = ((r - s) as f32 / n as f32, s as f32 / n as f32);
                let pos = p0 + (p1 - p0) * a + (p2 - p0) * b;
                let uv = c0.1 + (c1.1 - c0.1) * a + (c2.1 - c0.1) * b;
                (m.add_vertex(pos), uv)
            };
            row.push(c);
        }
        t.push(row);
    }
    let mut out = Vec::new();
    for r in 0..n {
        for s in 0..=r {
            out.push(vec![t[r][s], t[r + 1][s], t[r + 1][s + 1]]);
            if s < r {
                out.push(vec![t[r][s], t[r + 1][s + 1], t[r][s + 1]]);
            }
        }
    }
    out
}
