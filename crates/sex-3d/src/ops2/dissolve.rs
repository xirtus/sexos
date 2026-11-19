// SPDX-License-Identifier: GPL-3.0-only
//! Dissolve vertices / edges / faces, degenerate dissolve, edge collapse,
//! delete edge loop and un-subdivide.
use super::*;
use std::collections::BTreeSet;

/// Dissolve Vertices (Blender defaults: no face split, no boundary tear).
/// The faces around each selected vertex are joined into one n-gon and the
/// vertex is removed from it; a vertex joining exactly two wire edges is
/// replaced by one edge. Leaves the new faces selected.
pub fn dissolve_verts(m: &mut Mesh, verts: &[u32]) -> Selection {
    let sel: BTreeSet<u32> = verts.iter().copied().filter(|&v| (v as usize) < m.positions.len()).collect();
    if sel.is_empty() {
        return Selection::default();
    }
    let vf = vert_faces(m);
    let mut dsu = Dsu::new(m.faces.len());
    let mut touched = BTreeSet::new();
    for &v in &sel {
        let fs = &vf[v as usize];
        for &f in fs {
            touched.insert(f);
            dsu.union(fs[0], f);
        }
    }
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for &f in &touched {
        let r = dsu.find(f);
        groups.entry(r).or_default().push(f);
    }
    let mut remove = vec![false; m.faces.len()];
    let mut added = Vec::new();
    let mut roots: Vec<u32> = groups.keys().copied().collect();
    roots.sort_unstable();
    for r in roots {
        let g = &groups[&r];
        let Some(lp) = region_outline(m, g) else { continue };
        let lp: Vec<Corner> = lp.into_iter().filter(|c| !sel.contains(&c.0)).collect();
        for &f in g {
            remove[f as usize] = true;
        }
        if lp.len() >= 3 {
            added.push(face_from(&m.faces[g[0] as usize], &lp));
        }
    }
    retain_faces(m, &remove);
    let base = m.faces.len() as u32;
    let n_added = added.len() as u32;
    m.faces.extend(added);
    // Wire vertices between exactly two loose edges become one edge.
    let used: HashSet<u32> = m.faces.iter().flat_map(|f| f.verts.iter().copied()).collect();
    for &v in &sel {
        if used.contains(&v) {
            continue;
        }
        let es: Vec<usize> = (0..m.loose_edges.len()).filter(|&i| m.loose_edges[i].contains(&v)).collect();
        if es.len() == 2 {
            let other = |i: usize| if m.loose_edges[i][0] == v { m.loose_edges[i][1] } else { m.loose_edges[i][0] };
            let (a, b) = (other(es[0]), other(es[1]));
            m.loose_edges.retain(|e| !e.contains(&v));
            if a != b && !m.edges().iter().any(|e| *e == ek(a, b)) {
                m.loose_edges.push(ek(a, b));
            }
        } else if !es.is_empty() && vf[v as usize].is_empty() {
            // A wire end or junction: Blender removes the vertex with its edges.
            m.loose_edges.retain(|e| !e.contains(&v));
        }
    }
    // Selected vertices still in faces whose region could not be joined stay.
    let new_faces: Vec<u32> = (base..base + n_added).collect();
    compact(m);
    Selection::faces(m, new_faces)
}

/// Dissolve Edges: the two faces of each selected edge are joined (chains of
/// such joins make one face). With `use_verts` (Blender's default) the
/// selected edges' vertices that are left between only two edges are removed
/// too. Leaves the new faces selected.
pub fn dissolve_edges(m: &mut Mesh, edges: &[[u32; 2]], use_verts: bool) -> Selection {
    let ef = edge_faces(m);
    let mut dsu = Dsu::new(m.faces.len());
    let mut touched = BTreeSet::new();
    let mut ends: Vec<u32> = Vec::new();
    for e in edges {
        let k = ekey(e[0], e[1]);
        let Some(fs) = ef.get(&k) else { continue };
        if fs.len() == 2 && fs[0] != fs[1] {
            dsu.union(fs[0], fs[1]);
            touched.insert(fs[0]);
            touched.insert(fs[1]);
            ends.push(k.0);
            ends.push(k.1);
        }
    }
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for &f in &touched {
        let r = dsu.find(f);
        groups.entry(r).or_default().push(f);
    }
    let mut roots: Vec<u32> = groups.keys().copied().collect();
    roots.sort_unstable();
    let gl: Vec<Vec<u32>> = roots.iter().map(|r| groups[r].clone()).collect();
    let (new_faces, _) = join_groups(m, &gl);
    if use_verts {
        dissolve_valence2(m, &ends);
    }
    compact(m);
    Selection::faces(m, new_faces)
}

/// Delete Edge Loop: dissolve the selected edges and the vertices they leave
/// between two edges. Nothing stays selected.
pub fn delete_edgeloop(m: &mut Mesh, edges: &[[u32; 2]]) -> Selection {
    dissolve_edges(m, edges, true);
    Selection::default()
}

/// Dissolve Faces: each connected group of selected faces (sharing edges)
/// becomes one n-gon. `use_verts` also removes the vertices left between two
/// edges on its outline (Blender's default is off).
pub fn dissolve_faces(m: &mut Mesh, faces: &[u32], use_verts: bool) -> Selection {
    let sel: BTreeSet<u32> = faces.iter().copied().filter(|&f| (f as usize) < m.faces.len()).collect();
    let ef = edge_faces(m);
    let mut dsu = Dsu::new(m.faces.len());
    for fs in ef.values() {
        if fs.len() == 2 && sel.contains(&fs[0]) && sel.contains(&fs[1]) {
            dsu.union(fs[0], fs[1]);
        }
    }
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for &f in &sel {
        let r = dsu.find(f);
        groups.entry(r).or_default().push(f);
    }
    let mut roots: Vec<u32> = groups.keys().copied().collect();
    roots.sort_unstable();
    let gl: Vec<Vec<u32>> = roots.iter().map(|r| groups[r].clone()).collect();
    let singles: Vec<u32> = gl.iter().filter(|g| g.len() == 1).map(|g| g[0]).collect();
    let (mut new_faces, map) = join_groups(m, &gl);
    // Lone faces (and groups that could not be joined) stay selected.
    for g in &gl {
        for &f in g {
            if map[f as usize] != NONE && (g.len() > 1 || singles.contains(&f)) {
                new_faces.push(map[f as usize]);
            }
        }
    }
    if use_verts {
        let outline: Vec<u32> = new_faces.iter().flat_map(|&f| m.faces[f as usize].verts.clone()).collect();
        dissolve_valence2(m, &outline);
    }
    compact(m);
    Selection::faces(m, new_faces)
}

/// Degenerate Dissolve: within the selected vertices, collapse edges shorter
/// than `dist` and dissolve faces whose area is (almost) zero into their
/// neighbour across their longest edge. Returns the selection, remapped.
pub fn dissolve_degenerate(m: &mut Mesh, verts: &[u32], dist: f32) -> Selection {
    let sel: HashSet<u32> = verts.iter().copied().collect();
    // 1. Short edges.
    let mut dsu = Dsu::new(m.positions.len());
    let mut any = false;
    for e in m.edges() {
        if sel.contains(&e[0]) && sel.contains(&e[1]) && m.positions[e[0] as usize].distance(m.positions[e[1] as usize]) <= dist {
            dsu.union(e[0], e[1]);
            any = true;
        }
    }
    if any {
        let n = m.positions.len();
        let remap: Vec<u32> = (0..n as u32).map(|v| dsu.find(v)).collect();
        let mut sum: HashMap<u32, (Vec3, f32)> = HashMap::new();
        for v in 0..n {
            let e = sum.entry(remap[v]).or_insert((Vec3::ZERO, 0.0));
            e.0 += m.positions[v];
            e.1 += 1.0;
        }
        for (&r, &(s, c)) in &sum {
            if c > 1.0 {
                m.positions[r as usize] = s / c;
            }
        }
        m.remap_vertices(&remap);
    }
    // 2. Zero-area faces fold into the neighbour across their longest edge.
    for _ in 0..m.faces.len() + 1 {
        let found = (0..m.faces.len()).find(|&f| {
            let face = &m.faces[f];
            if !face.verts.iter().all(|v| sel.contains(v)) {
                return false;
            }
            let longest = face.edges().map(|(a, b)| m.positions[a as usize].distance(m.positions[b as usize])).fold(0.0f32, f32::max);
            m.face_area(f) <= dist * longest.max(dist)
        });
        let Some(f) = found else { break };
        let face = m.faces[f].clone();
        let n = face.verts.len();
        let i = (0..n)
            .max_by(|&i, &j| {
                let l = |k: usize| m.positions[face.verts[k] as usize].distance(m.positions[face.verts[(k + 1) % n] as usize]);
                l(i).total_cmp(&l(j))
            })
            .unwrap_or(0);
        let (a, b) = (face.verts[i], face.verts[(i + 1) % n]);
        // Corners strictly between b and a going forward from b.
        let path: Vec<u32> = (2..n).map(|k| face.verts[(i + k) % n]).collect();
        let nb = (0..m.faces.len()).find(|&g| g != f && m.faces[g].edges().any(|e| e == (b, a)));
        if let Some(g) = nb {
            let gf = &mut m.faces[g];
            let k = gf.verts.len();
            let at = (0..k).find(|&c| gf.verts[c] == b && gf.verts[(c + 1) % k] == a).unwrap_or(0);
            let has_uv = gf.uvs.len() == k;
            let (ub, ua) = if has_uv { (gf.uvs[at], gf.uvs[(at + 1) % k]) } else { (Vec2::ZERO, Vec2::ZERO) };
            for (j, &v) in path.iter().enumerate() {
                gf.verts.insert(at + 1 + j, v);
                if has_uv {
                    gf.uvs.insert(at + 1 + j, ub.lerp(ua, (j + 1) as f32 / (path.len() + 1) as f32));
                }
            }
        }
        m.faces.remove(f);
    }
    let remap = compact(m);
    Selection::verts(remap_list(verts, &remap))
}

/// Edge Collapse: each connected group of selected edges becomes one vertex
/// at the group's centre. Leaves the merged vertices selected.
pub fn edge_collapse(m: &mut Mesh, edges: &[[u32; 2]]) -> Selection {
    let mut dsu = Dsu::new(m.positions.len());
    let mut members: BTreeSet<u32> = BTreeSet::new();
    for e in edges {
        if (e[0] as usize) < m.positions.len() && (e[1] as usize) < m.positions.len() && e[0] != e[1] {
            dsu.union(e[0], e[1]);
            members.insert(e[0]);
            members.insert(e[1]);
        }
    }
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for &v in &members {
        let r = dsu.find(v);
        groups.entry(r).or_default().push(v);
    }
    let mut remap: Vec<u32> = (0..m.positions.len() as u32).collect();
    let mut roots = Vec::new();
    for (&r, g) in &groups {
        let c = g.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / g.len() as f32;
        m.positions[r as usize] = c;
        for &v in g {
            remap[v as usize] = r;
        }
        roots.push(r);
    }
    m.remap_vertices(&remap);
    ops_remove_collapsed(m);
    let rm = compact(m);
    Selection::verts(remap_list(&roots, &rm))
}

/// Faces left with a repeated vertex after a collapse lose the repeat (the
/// pinched part is a zero-area spike); faces below three corners go.
fn ops_remove_collapsed(m: &mut Mesh) {
    for f in &mut m.faces {
        let mut seen = HashSet::new();
        if f.verts.iter().all(|v| seen.insert(*v)) {
            continue;
        }
        // Keep the first occurrence of each vertex.
        let mut seen = HashSet::new();
        let cs: Vec<Corner> = corners(f).into_iter().filter(|c| seen.insert(c.0)).collect();
        *f = face_from(f, &cs);
    }
    m.faces.retain(|f| f.verts.len() >= 3);
    m.loose_edges.retain(|e| e[0] != e[1]);
}

/// Un-Subdivide (Blender's default 2 iterations): undo grid subdivision in
/// the selected region. Vertices at the centre of a 2x2 block of quads are
/// dissolved (their four faces join), then the vertices left between two
/// edges in the middle of the old edges go too.
pub fn unsubdivide(m: &mut Mesh, verts: &[u32], iterations: u32) -> Selection {
    let mut sel: Vec<u32> = verts.to_vec();
    for _ in 0..iterations.max(1) {
        if !unsubdivide_once(m, &mut sel) {
            break;
        }
    }
    Selection::verts(sel)
}

fn unsubdivide_once(m: &mut Mesh, sel: &mut Vec<u32>) -> bool {
    let nv = m.positions.len();
    let selset: HashSet<u32> = sel.iter().copied().collect();
    let adj = adjacency(m);
    let vf = vert_faces(m);
    let ef = edge_faces(m);
    let boundary: Vec<bool> = (0..nv).map(|v| adj[v].iter().any(|&u| ef.get(&ekey(v as u32, u)).map_or(true, |f| f.len() != 2))).collect();
    let grid_vert = |v: u32| -> bool {
        let v = v as usize;
        adj[v].len() == 4 && vf[v].len() == 4 && !boundary[v] && vf[v].iter().all(|&f| m.faces[f as usize].verts.len() == 4)
    };
    let eligible = |v: u32| selset.contains(&v) && grid_vert(v);
    // Opposite corner of v in quad f.
    let diag = |f: u32, v: u32| -> u32 {
        let fv = &m.faces[f as usize].verts;
        let i = fv.iter().position(|&x| x == v).unwrap_or(0);
        fv[(i + 2) % 4]
    };
    // The vertex continuing straight on from v through m (valence-4 m).
    let straight = |v: u32, mid: u32| -> Option<u32> {
        if !grid_vert(mid) {
            return None;
        }
        let near: HashSet<u32> = vf[mid as usize].iter().filter(|&&f| m.faces[f as usize].verts.contains(&v)).flat_map(|&f| m.faces[f as usize].verts.iter().copied()).collect();
        adj[mid as usize].iter().copied().find(|u| !near.contains(u))
    };
    // 0 unknown, 1 collapse, 2 mid-edge, 3 keep.
    let mut state = vec![0u8; nv];
    let mut seeds: Vec<u32> = Vec::new();
    for v in 0..nv as u32 {
        if eligible(v) && vf[v as usize].iter().any(|&f| {
            let d = diag(f, v);
            adj[d as usize].len() != 4 || boundary[d as usize]
        }) {
            seeds.push(v);
        }
    }
    seeds.extend((0..nv as u32).filter(|&v| eligible(v)));
    let mut any = false;
    for s in seeds {
        if state[s as usize] != 0 {
            continue;
        }
        state[s as usize] = 1;
        any = true;
        let mut stack = vec![s];
        while let Some(v) = stack.pop() {
            for &f in &vf[v as usize] {
                let d = diag(f, v);
                if state[d as usize] == 0 {
                    state[d as usize] = 3;
                }
            }
            for &mid in &adj[v as usize] {
                if state[mid as usize] == 0 {
                    state[mid as usize] = 2;
                }
                if let Some(w) = straight(v, mid) {
                    if state[w as usize] == 0 && eligible(w) {
                        state[w as usize] = 1;
                        stack.push(w);
                    }
                }
            }
        }
    }
    if !any {
        return false;
    }
    let collapse: Vec<u32> = (0..nv as u32).filter(|&v| state[v as usize] == 1).collect();
    let mids: Vec<u32> = (0..nv as u32).filter(|&v| state[v as usize] == 2).collect();
    let mut dsu = Dsu::new(m.faces.len());
    let mut touched = BTreeSet::new();
    for &v in &collapse {
        let fs = &vf[v as usize];
        for &f in fs {
            dsu.union(fs[0], f);
            touched.insert(f);
        }
    }
    let mut groups: HashMap<u32, Vec<u32>> = HashMap::new();
    for &f in &touched {
        let r = dsu.find(f);
        groups.entry(r).or_default().push(f);
    }
    let cset: HashSet<u32> = collapse.iter().copied().collect();
    let mut remove = vec![false; m.faces.len()];
    let mut added = Vec::new();
    for g in groups.values() {
        if let Some(lp) = region_outline(m, g) {
            let lp: Vec<Corner> = lp.into_iter().filter(|c| !cset.contains(&c.0)).collect();
            if lp.len() >= 3 {
                added.push(face_from(&m.faces[g[0] as usize], &lp));
                for &f in g {
                    remove[f as usize] = true;
                }
            }
        }
    }
    retain_faces(m, &remove);
    m.faces.extend(added);
    dissolve_valence2(m, &mids);
    let remap = compact(m);
    *sel = remap_list(sel, &remap);
    true
}
