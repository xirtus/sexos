// SPDX-License-Identifier: GPL-3.0-only
//! Topology queries behind the selection operators.
use super::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Select Non-Manifold (Blender's defaults: wire, boundaries, multiple
/// faces, non-contiguous and vertices all on): edges that are wire, on a
/// boundary, used by more than two faces, or between two faces wound the
/// same way; and vertices whose faces do not form one fan.
pub fn non_manifold(m: &Mesh) -> Selection {
    let ef = edge_faces(m);
    let mut dcount: HashMap<(u32, u32), u32> = HashMap::new();
    for f in &m.faces {
        for e in f.edges() {
            *dcount.entry(e).or_insert(0) += 1;
        }
    }
    let mut edges: Vec<[u32; 2]> = Vec::new();
    for (k, fs) in &ef {
        let same_way = dcount.get(k).copied().unwrap_or(0) > 1 || dcount.get(&(k.1, k.0)).copied().unwrap_or(0) > 1;
        if fs.len() != 2 || same_way {
            edges.push([k.0, k.1]);
        }
    }
    for e in &m.loose_edges {
        if !ef.contains_key(&ekey(e[0], e[1])) {
            edges.push(ek(e[0], e[1]));
        }
    }
    let mut s = Selection::edges(edges);
    let dir = dir_map(m);
    let vf = vert_faces(m);
    for v in 0..m.positions.len() as u32 {
        if !vf[v as usize].is_empty() && vertex_fan(m, &dir, &vf[v as usize], v).is_none() {
            s.verts.push(v);
        }
    }
    s.verts.sort_unstable();
    s.verts.dedup();
    s
}

/// For each vertex, the vertex at its mirror position across the plane
/// normal to `axis` through the origin (within `eps`), if any.
pub fn mirror_vertex_map(m: &Mesh, axis: usize, eps: f32) -> Vec<Option<u32>> {
    let cell = eps.max(1e-9);
    let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
    for (i, &p) in m.positions.iter().enumerate() {
        grid.entry(key(p)).or_default().push(i as u32);
    }
    m.positions
        .iter()
        .map(|&p| {
            let mut q = p;
            match axis {
                0 => q.x = -q.x,
                1 => q.y = -q.y,
                _ => q.z = -q.z,
            }
            let k = key(q);
            let mut best: Option<(u32, f32)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(l) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                            for &u in l {
                                let d = m.positions[u as usize].distance(q);
                                if d <= eps && best.map_or(true, |b| d < b.1) {
                                    best = Some((u, d));
                                }
                            }
                        }
                    }
                }
            }
            best.map(|b| b.0)
        })
        .collect()
}

/// Dijkstra over a graph given by a neighbour function; returns the path.
fn dijkstra(n: usize, a: u32, b: u32, nbrs: &dyn Fn(u32) -> Vec<(u32, f32)>) -> Option<Vec<u32>> {
    let mut dist = vec![f32::INFINITY; n];
    let mut prev = vec![NONE; n];
    let mut heap = BinaryHeap::new();
    dist[a as usize] = 0.0;
    heap.push(Reverse((0u32, a)));
    while let Some(Reverse((dbits, v))) = heap.pop() {
        let d = f32::from_bits(dbits);
        if d > dist[v as usize] {
            continue;
        }
        if v == b {
            break;
        }
        for (u, w) in nbrs(v) {
            let nd = d + w.max(0.0);
            if nd < dist[u as usize] {
                dist[u as usize] = nd;
                prev[u as usize] = v;
                // Non-negative f32 bit patterns sort like the values.
                heap.push(Reverse((nd.to_bits(), u)));
            }
        }
    }
    if !dist[b as usize].is_finite() {
        return None;
    }
    let mut path = vec![b];
    let mut cur = b;
    while cur != a {
        cur = prev[cur as usize];
        path.push(cur);
    }
    path.reverse();
    Some(path)
}

/// Shortest path between two vertices along edges (by length), as Select
/// Shortest Path uses in vertex and edge mode.
pub fn shortest_path_verts(m: &Mesh, a: u32, b: u32) -> Option<Vec<u32>> {
    let n = m.positions.len();
    if a as usize >= n || b as usize >= n {
        return None;
    }
    let adj = adjacency(m);
    dijkstra(n, a, b, &|v| adj[v as usize].iter().map(|&u| (u, m.positions[v as usize].distance(m.positions[u as usize]))).collect())
}

/// Shortest path between two faces across shared edges (by distance between
/// face centres), as Select Shortest Path uses in face mode.
pub fn shortest_path_faces(m: &Mesh, a: u32, b: u32) -> Option<Vec<u32>> {
    let n = m.faces.len();
    if a as usize >= n || b as usize >= n {
        return None;
    }
    let ef = edge_faces(m);
    let centers: Vec<Vec3> = (0..n).map(|f| m.face_center(f)).collect();
    dijkstra(n, a, b, &|f| {
        let mut out = Vec::new();
        for (x, y) in m.faces[f as usize].edges() {
            for &g in &ef[&ekey(x, y)] {
                if g != f {
                    out.push((g, centers[f as usize].distance(centers[g as usize])));
                }
            }
        }
        out
    })
}

/// Select Interior Faces: faces all of whose edges are used by more than two
/// faces.
pub fn interior_faces(m: &Mesh) -> Vec<u32> {
    let ef = edge_faces(m);
    (0..m.faces.len() as u32).filter(|&f| m.faces[f as usize].edges().all(|(a, b)| ef[&ekey(a, b)].len() > 2)).collect()
}

/// The edge ring through `edge`: from each quad on it to the opposite edge,
/// on through the next quad, in both directions, stopping at non-quads,
/// boundaries and where the ring closes.
pub fn edge_ring(m: &Mesh, edge: [u32; 2]) -> Vec<[u32; 2]> {
    let ef = edge_faces(m);
    let start = ekey(edge[0], edge[1]);
    let Some(fs) = ef.get(&start) else { return Vec::new() };
    let mut out = vec![[start.0, start.1]];
    let mut seen: HashSet<(u32, u32)> = HashSet::from([start]);
    let mut seen_faces: HashSet<u32> = HashSet::new();
    for &f0 in fs.iter().take(2) {
        let mut e = start;
        let mut f = f0;
        loop {
            if !seen_faces.insert(f) {
                break;
            }
            let fv = &m.faces[f as usize].verts;
            if fv.len() != 4 {
                break;
            }
            let Some(i) = (0..4).find(|&i| ekey(fv[i], fv[(i + 1) % 4]) == e) else { break };
            let opp = ekey(fv[(i + 2) % 4], fv[(i + 3) % 4]);
            if !seen.insert(opp) {
                break;
            }
            out.push([opp.0, opp.1]);
            e = opp;
            match ef[&opp].iter().copied().find(|&g| g != f) {
                Some(g) => f = g,
                None => break,
            }
        }
    }
    out
}
