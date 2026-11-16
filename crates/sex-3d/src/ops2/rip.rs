// SPDX-License-Identifier: GPL-3.0-only
//! Rip (V), Rip and Fill, Rip and Extend (Alt+D).
use super::*;

/// What [`rip`] did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RipResult {
    /// The torn-off side, selected afterwards (Blender moves it with the grab).
    pub selection: Selection,
    /// Faces added by the fill option.
    pub fill_faces: Vec<u32>,
}

/// Rip: tear the mesh open at the selected vertices. Along selected edges
/// (each with two faces) the faces on the left of the edge path get copies of
/// the vertices; at a vertex where the path ends, or a lone selected vertex,
/// the fan of faces is split in two halves. Blender picks the side under the
/// mouse; without a pointer the left side of the path is torn off (and for a
/// lone vertex, the second half of its fan). With `fill` the gap is closed
/// with new faces, like Rip Vertices and Fill. Returns the torn-off vertices
/// as the selection.
pub fn rip(m: &mut Mesh, verts: &[u32], edges: &[[u32; 2]], fill: bool) -> RipResult {
    let vsel: HashSet<u32> = verts.iter().copied().filter(|&v| (v as usize) < m.positions.len()).collect();
    let ef = edge_faces(m);
    let esel: HashSet<(u32, u32)> = edges.iter().map(|e| ekey(e[0], e[1])).filter(|k| vsel.contains(&k.0) && vsel.contains(&k.1) && ef.get(k).map_or(false, |f| f.len() == 2)).collect();
    let dir = dir_map(m);
    let vf = vert_faces(m);
    // Faces on the left of the selected edge paths: the face holding the
    // edge in path direction. Paths are oriented by walking them.
    let mut left: HashSet<u32> = HashSet::new();
    {
        let mut nb: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(a, b) in &esel {
            nb.entry(a).or_default().push(b);
            nb.entry(b).or_default().push(a);
        }
        let mut done: HashSet<(u32, u32)> = HashSet::new();
        // Path ends first so open paths are walked from one end.
        let mut sorted: Vec<u32> = nb.keys().copied().collect();
        sorted.sort_by_key(|&v| (nb[&v].len() != 1, v));
        for s in sorted {
            let mut cur = s;
            loop {
                let Some(&nx) = nb[&cur].iter().find(|&&u| !done.contains(&ekey(cur, u))) else { break };
                done.insert(ekey(cur, nx));
                if let Some(&f) = dir.get(&(cur, nx)) {
                    left.insert(f);
                }
                cur = nx;
            }
        }
    }
    // (face, corner of v) -> new vertex.
    let mut assign: Vec<(u32, u32, u32)> = Vec::new();
    let mut moved = Vec::new();
    // Cut edges: (v, spoke u, face A, face B) in original terms.
    let mut cuts: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut sorted_v: Vec<u32> = vsel.iter().copied().collect();
    sorted_v.sort_unstable();
    for &v in &sorted_v {
        let Some(fan) = vertex_fan(m, &dir, &vf[v as usize], v) else { continue };
        let k = fan.faces.len();
        // Spoke i lies between faces i and i+1 (wrapping when closed).
        let n_spokes = if fan.closed { k } else { k - 1 };
        let mut cut: Vec<usize> = (0..n_spokes).filter(|&i| esel.contains(&ekey(v, fan.spokes[i]))).collect();
        if cut.is_empty() {
            if k < 2 {
                continue;
            }
            if fan.closed {
                cut = vec![k - 1, (k - 1 + k / 2) % k];
            } else {
                cut = vec![(k - 1) / 2];
            }
        } else if cut.len() == 1 && fan.closed {
            cut.push((cut[0] + k / 2) % k);
        }
        cut.sort_unstable();
        cut.dedup();
        // Components: runs of faces between cuts.
        let mut comp = vec![0usize; k];
        let mut c = 0;
        let start = if fan.closed { (cut[0] + 1) % k } else { 0 };
        for step in 0..k {
            let i = (start + step) % k;
            comp[i] = c;
            let spoke = i; // spoke after face i
            if spoke < n_spokes && cut.contains(&spoke) {
                c += 1;
            }
        }
        let ncomp = if fan.closed { c } else { c + 1 };
        if ncomp < 2 {
            continue;
        }
        let has_left: Vec<bool> = (0..ncomp).map(|ci| (0..k).any(|i| comp[i] == ci && left.contains(&fan.faces[i]))).collect();
        let mut move_comp: Vec<usize> = (0..ncomp).filter(|&ci| has_left[ci]).collect();
        if move_comp.is_empty() || move_comp.len() == ncomp {
            move_comp = (1..ncomp).collect();
        }
        for &ci in &move_comp {
            let nv = m.copy_vertex(v);
            moved.push(nv);
            for i in 0..k {
                if comp[i] == ci {
                    assign.push((fan.faces[i], v, nv));
                }
            }
        }
        for &s in &cut {
            let (fa, fb) = (fan.faces[s], fan.faces[(s + 1) % k]);
            cuts.push((v, fan.spokes[s], fa, fb));
        }
    }
    // Where each cut edge's corners sit, before reassigning.
    let pos_of = |m: &Mesh, f: u32, x: u32| m.faces[f as usize].verts.iter().position(|&y| y == x);
    let mut cut_info = Vec::new();
    let mut seen = HashSet::new();
    for &(v, u, fa, fb) in &cuts {
        if !seen.insert(ekey(v, u)) {
            continue;
        }
        if let (Some(av), Some(au), Some(bv), Some(bu)) = (pos_of(m, fa, v), pos_of(m, fa, u), pos_of(m, fb, v), pos_of(m, fb, u)) {
            cut_info.push((fa, av, au, fb, bv, bu));
        }
    }
    for &(f, v, nv) in &assign {
        for x in &mut m.faces[f as usize].verts {
            if *x == v {
                *x = nv;
            }
        }
    }
    let mut fill_faces = Vec::new();
    if fill {
        for &(fa, av, au, fb, bv, bu) in &cut_info {
            let (xa, ya) = (m.faces[fa as usize].verts[av], m.faces[fa as usize].verts[au]);
            let (xb, yb) = (m.faces[fb as usize].verts[bv], m.faces[fb as usize].verts[bu]);
            if xa == xb && ya == yb {
                continue;
            }
            // Face A runs (p -> q) along the cut; the fill runs it backwards.
            let k = m.faces[fa as usize].verts.len();
            let (p, q, p2, q2) = if (av + 1) % k == au { (xa, ya, xb, yb) } else { (ya, xa, yb, xb) };
            let mut lp = vec![q, p, p2, q2];
            lp.dedup();
            if lp.first() == lp.last() {
                lp.pop();
            }
            if lp.len() >= 3 {
                let src = m.faces[fa as usize].clone();
                m.faces.push(Face { verts: lp, uvs: Vec::new(), material: src.material, smooth: src.smooth });
                fill_faces.push(m.faces.len() as u32 - 1);
            }
        }
        crate::ops::fill_missing_uvs(m);
    }
    RipResult { selection: Selection::verts(moved), fill_faces }
}

/// Rip Vertices and Extend: each selected vertex gets a new vertex inserted
/// next to it along one of its boundary edges (any edge if it has none; a
/// new wire edge if it has no faces). The new vertices are returned
/// selected, ready to be moved.
pub fn rip_edge(m: &mut Mesh, verts: &[u32]) -> Selection {
    let ef = edge_faces(m);
    let adj = adjacency(m);
    let mut out = Vec::new();
    for &v in verts {
        if v as usize >= m.positions.len() {
            continue;
        }
        let nbrs = &adj[v as usize];
        let face_nbrs: Vec<u32> = nbrs.iter().copied().filter(|&u| ef.contains_key(&ekey(v, u))).collect();
        let pick = face_nbrs.iter().copied().find(|&u| ef[&ekey(v, u)].len() == 1).or_else(|| face_nbrs.first().copied());
        let nv = m.copy_vertex(v);
        match pick {
            Some(u) => insert_edge_vertex(m, v, u, nv, 0.0),
            None => m.loose_edges.push([v, nv]),
        }
        out.push(nv);
    }
    Selection::verts(out)
}
