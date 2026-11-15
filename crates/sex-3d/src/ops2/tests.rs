// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::ops::{self as ops1, BooleanOp};
use crate::primitives;

/// (vertices, edges, faces)
fn counts(m: &Mesh) -> (usize, usize, usize) {
    (m.vertex_count(), m.edges().len(), m.face_count())
}

fn closed(m: &Mesh) -> bool {
    m.validate().is_ok() && m.half_edges().is_closed_manifold()
}

fn all_verts(m: &Mesh) -> Vec<u32> {
    (0..m.positions.len() as u32).collect()
}

fn all_faces(m: &Mesh) -> Vec<u32> {
    (0..m.faces.len() as u32).collect()
}

fn has_edge(m: &Mesh, a: u32, b: u32) -> bool {
    m.edges().contains(&ek(a, b))
}

fn boundary_edges(m: &Mesh) -> usize {
    m.half_edges().boundary_half_edges().len()
}

/// Edges of the face in `faces` order.
fn face_edges(m: &Mesh, faces: &[u32]) -> Vec<[u32; 2]> {
    Selection::faces(m, faces.to_vec()).edges
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

// ---------------------------------------------------------------------------
// Dissolve

#[test]
fn dissolve_verts_joins_faces_and_removes_vertex() {
    let mut g = primitives::grid(2, 2, 2.0);
    let s = dissolve_verts(&mut g, &[4]);
    assert_eq!(counts(&g), (8, 8, 1));
    assert_eq!(g.faces[0].len(), 8);
    assert_eq!(s.faces, vec![0]);
    assert!(g.has_uvs());
    // Boundary vertex: two quads become a pentagon.
    let mut g = primitives::grid(2, 2, 2.0);
    dissolve_verts(&mut g, &[1]);
    assert_eq!((g.vertex_count(), g.face_count()), (8, 3));
    assert!(g.faces.iter().any(|f| f.len() == 5));
    // Cube corner: three quads become one hexagon, still closed.
    let mut c = primitives::cube(2.0);
    dissolve_verts(&mut c, &[0]);
    assert_eq!((c.vertex_count(), c.face_count()), (7, 4));
    assert!(closed(&c));
    // A wire vertex between two edges becomes one edge.
    let mut w = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(2.0, 0.0, 0.0)], vec![]);
    w.loose_edges = vec![[0, 1], [1, 2]];
    dissolve_verts(&mut w, &[1]);
    assert_eq!(w.loose_edges, vec![[0, 1]]);
    assert_eq!(w.vertex_count(), 2);
}

#[test]
fn dissolve_edges_and_delete_edge_loop() {
    let mut g = primitives::grid(2, 2, 2.0);
    let s = dissolve_edges(&mut g, &[[1, 4]], true);
    // Vertex 1 is left between two boundary edges and goes; vertex 4 stays.
    assert_eq!((g.vertex_count(), g.face_count()), (8, 3));
    assert_eq!(s.faces.len(), 1);
    assert_eq!(g.faces[s.faces[0] as usize].len(), 5);
    // Loop cut a cube and delete the loop again: back to a cube.
    let mut c = primitives::cube(2.0);
    let new = ops1::loop_cut(&mut c, [0, 1], 1);
    assert_eq!(counts(&c), (12, 20, 10));
    let set: HashSet<u32> = new.iter().copied().collect();
    let loop_edges: Vec<[u32; 2]> = c.edges().into_iter().filter(|e| set.contains(&e[0]) && set.contains(&e[1])).collect();
    assert_eq!(loop_edges.len(), 4);
    let s = delete_edgeloop(&mut c, &loop_edges);
    assert_eq!(counts(&c), (8, 12, 6));
    assert!(closed(&c));
    assert!(s.faces.is_empty());
    assert!((c.volume() - 8.0).abs() < 1e-4);
}

#[test]
fn dissolve_faces_makes_one_ngon() {
    let mut g = primitives::grid(3, 3, 3.0);
    let af = all_faces(&g);
    let s = dissolve_faces(&mut g, &af, false);
    assert_eq!(counts(&g), (12, 12, 1));
    assert_eq!(s.faces, vec![0]);
    // Two separate groups stay separate.
    let mut g = primitives::grid(3, 1, 3.0);
    let s = dissolve_faces(&mut g, &[0, 2], false);
    assert_eq!(g.face_count(), 3);
    assert_eq!(s.faces.len(), 2);
}

#[test]
fn dissolve_degenerate_collapses_short_edges_and_slivers() {
    // A quad with a doubled corner.
    let mut m = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(1.0, 1.0, 0.0), v(0.0, 1.0, 0.0), v(1.0, 0.0, 0.0)], vec![vec![0, 1, 4, 2, 3]]);
    let av = all_verts(&m);
    dissolve_degenerate(&mut m, &av, 1e-4);
    assert_eq!(counts(&m), (4, 4, 1));
    // A zero-area triangle folds into its neighbour, which gains its middle vertex.
    let mut m = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(2.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(1.0, 1.0, 0.0)], vec![vec![0, 2, 1], vec![0, 1, 3]]);
    let av = all_verts(&m);
    dissolve_degenerate(&mut m, &av, 1e-4);
    assert_eq!(m.face_count(), 1);
    assert_eq!(m.faces[0].verts, vec![0, 2, 1, 3]);
}

#[test]
fn edge_collapse_merges_to_midpoint() {
    let mut c = primitives::cube(2.0);
    let (p0, p1) = (c.positions[0], c.positions[1]);
    let s = edge_collapse(&mut c, &[[0, 1]]);
    assert_eq!(c.vertex_count(), 7);
    assert_eq!(c.face_count(), 6);
    assert!(closed(&c));
    assert_eq!(s.verts.len(), 1);
    assert!(c.positions[s.verts[0] as usize].approx_eq((p0 + p1) * 0.5, 1e-6));
}

// ---------------------------------------------------------------------------
// Subdivision

#[test]
fn subdivide_patterns() {
    let mut p = primitives::plane(2.0);
    let e = p.edges();
    subdivide(&mut p, &e, 1);
    assert_eq!(counts(&p), (9, 12, 4));
    assert!(p.positions.iter().any(|q| q.approx_eq(Vec3::ZERO, 1e-6)));
    assert!(p.has_uvs());
    let mut p = primitives::plane(2.0);
    let e = p.edges();
    let s = subdivide(&mut p, &e, 2);
    assert_eq!((p.vertex_count(), p.face_count()), (16, 9));
    assert_eq!(s.verts.len(), 16);
    // Cube, every edge: 26 vertices, 24 quads, closed.
    let mut c = primitives::cube(2.0);
    let e = c.edges();
    subdivide(&mut c, &e, 1);
    assert_eq!(counts(&c), (26, 48, 24));
    assert!(closed(&c));
    assert!((c.volume() - 8.0).abs() < 1e-4);
    // Triangle: all edges -> 4, one edge -> a fan of 2.
    let tri = || Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)], vec![vec![0, 1, 2]]);
    let mut t = tri();
    subdivide(&mut t, &[[0, 1], [1, 2], [2, 0]], 1);
    assert_eq!(counts(&t), (6, 9, 4));
    let mut t = tri();
    subdivide(&mut t, &[[0, 1], [1, 2], [2, 0]], 2);
    assert_eq!((t.vertex_count(), t.face_count()), (10, 9));
    assert!((t.surface_area() - 0.5).abs() < 1e-5);
    let mut t = tri();
    subdivide(&mut t, &[[0, 1]], 1);
    assert_eq!(t.face_count(), 2);
    // Quad: two adjacent edges are cut straight (a corner triangle and a
    // pentagon); one edge just gains a vertex.
    let mut q = primitives::plane(2.0);
    subdivide(&mut q, &[[0, 1], [1, 3]], 1);
    assert_eq!(q.face_count(), 2);
    let mut lens: Vec<usize> = q.faces.iter().map(|f| f.len()).collect();
    lens.sort_unstable();
    assert_eq!(lens, vec![3, 5]);
    let mut q = primitives::plane(2.0);
    subdivide(&mut q, &[[0, 1]], 1);
    assert_eq!(q.face_count(), 1);
    assert_eq!(q.faces[0].len(), 5);
}

#[test]
fn subdivide_edgering_cuts_strips() {
    let mut g = primitives::grid(3, 1, 3.0);
    // The four edges across the strip (x = const).
    let ring: Vec<[u32; 2]> = (0..4).map(|i| [i, i + 4]).collect();
    let s = subdivide_edgering(&mut g, &ring, 2, 1.0);
    assert_eq!((g.vertex_count(), g.face_count()), (16, 9));
    assert!(g.positions.iter().all(|p| p.z.abs() < 1e-6));
    // New vertices stay on their (straight) edges.
    for &nv in &s.verts {
        let p = g.positions[nv as usize];
        assert!((0..4).any(|i| (p.x - (-1.5 + i as f32)).abs() < 1e-5), "{p:?}");
    }
    assert_eq!(boundary_edges(&g), 12);
}

#[test]
fn unsubdivide_reverses_grid_subdivision() {
    let mut g = primitives::grid(4, 4, 2.0);
    let av = all_verts(&g);
    unsubdivide(&mut g, &av, 1);
    assert_eq!(counts(&g), (9, 12, 4));
    assert!(g.faces.iter().all(|f| f.len() == 4));
    let mut g = primitives::grid(4, 4, 2.0);
    let av = all_verts(&g);
    unsubdivide(&mut g, &av, 2);
    assert_eq!(counts(&g), (4, 4, 1));
    // A subdivided cube goes back to the cube.
    let mut c = primitives::cube(2.0);
    let e = c.edges();
    subdivide(&mut c, &e, 1);
    let av = all_verts(&c);
    unsubdivide(&mut c, &av, 1);
    assert_eq!(counts(&c), (8, 12, 6));
    assert!(closed(&c));
}

#[test]
fn poke_fans_from_weighted_centre() {
    let mut c = primitives::cube(2.0);
    let center = c.face_center(0);
    let s = poke(&mut c, &[0], 0.0);
    assert_eq!(counts(&c), (9, 16, 9));
    assert!(closed(&c));
    assert!(c.positions[8].approx_eq(center, 1e-6));
    assert_eq!(s.faces.len(), 4);
    assert!((c.volume() - 8.0).abs() < 1e-4);
}

// ---------------------------------------------------------------------------
// Triangles

fn triangulated(m: &Mesh) -> Mesh {
    let mut out = Mesh { positions: m.positions.clone(), faces: Vec::new(), loose_edges: Vec::new() };
    for t in m.triangulate() {
        out.faces.push(Face::new(t.verts.to_vec()));
    }
    out
}

#[test]
fn tris_to_quads_rejoins_triangulated_quads() {
    let mut g = triangulated(&primitives::grid(2, 2, 2.0));
    assert_eq!(g.face_count(), 8);
    let lim = 40f32.to_radians();
    let af = all_faces(&g);
    let s = tris_to_quads(&mut g, &af, lim, lim);
    assert_eq!(counts(&g), (9, 12, 4));
    assert_eq!(s.faces.len(), 4);
    let mut c = triangulated(&primitives::cube(2.0));
    let af = all_faces(&c);
    tris_to_quads(&mut c, &af, lim, lim);
    assert_eq!(counts(&c), (8, 12, 6));
    assert!(closed(&c));
    // Across a 90 degree fold nothing joins.
    let mut f = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0), v(0.0, 0.0, 1.0)], vec![vec![0, 1, 2], vec![1, 0, 3]]);
    tris_to_quads(&mut f, &[0, 1], lim, lim);
    assert_eq!(f.face_count(), 2);
}

#[test]
fn beautify_flips_long_diagonal() {
    let mut m = Mesh::from_faces(vec![v(-2.0, 0.0, 0.0), v(2.0, 0.0, 0.0), v(0.0, 0.5, 0.0), v(0.0, -0.5, 0.0)], vec![vec![0, 1, 2], vec![1, 0, 3]]);
    let area = m.surface_area();
    beautify_fill(&mut m, &[0, 1]);
    assert!(has_edge(&m, 2, 3));
    assert!(!has_edge(&m, 0, 1));
    assert!((m.surface_area() - area).abs() < 1e-5);
    assert!((0..2).all(|f| m.face_normal(f).z > 0.99));
}

#[test]
fn edge_rotate_turns_shared_edge() {
    let mut g = primitives::grid(2, 1, 2.0);
    let s = edge_rotate(&mut g, &[[1, 4]], false);
    assert!(!has_edge(&g, 1, 4));
    assert_eq!(s.edges.len(), 1);
    let [a, b] = s.edges[0];
    assert!(has_edge(&g, a, b));
    assert_eq!(counts(&g), (6, 7, 2));
    assert!(g.faces.iter().all(|f| f.len() == 4));
    assert!((0..2).all(|f| g.face_normal(f).z > 0.99));
    // Counter-clockwise goes the other way.
    let mut h = primitives::grid(2, 1, 2.0);
    let s2 = edge_rotate(&mut h, &[[1, 4]], true);
    assert_ne!(s2.edges, s.edges);
}

// ---------------------------------------------------------------------------
// Split and rip

#[test]
fn edge_split_and_split() {
    let mut g = primitives::grid(2, 1, 2.0);
    let s = edge_split(&mut g, &[[1, 4]]);
    assert_eq!(g.vertex_count(), 8);
    assert_eq!(s.edges.len(), 2);
    assert_eq!(boundary_edges(&g), 8);
    // Splitting the cube's top loop detaches the top face.
    let mut c = primitives::cube(2.0);
    let top = c.faces.iter().position(|f| f.verts == vec![4, 5, 7, 6]).unwrap() as u32;
    let e = face_edges(&c, &[top]);
    edge_split(&mut c, &e);
    assert_eq!(c.vertex_count(), 12);
    assert_eq!(boundary_edges(&c), 8);
    let mut c = primitives::cube(2.0);
    let s = split(&mut c, &[0]);
    assert_eq!(c.vertex_count(), 12);
    assert_eq!(s.faces, vec![0]);
    assert_eq!(boundary_edges(&c), 8);
}

#[test]
fn rip_vertex_and_path() {
    // Lone interior vertex: torn along two opposite spokes.
    let mut g = primitives::grid(2, 2, 2.0);
    let r = rip(&mut g, &[4], &[], false);
    assert_eq!(g.vertex_count(), 10);
    assert_eq!(r.selection.verts.len(), 1);
    assert_eq!(boundary_edges(&g), 12);
    // An edge between two interior vertices, with fill: the gap is closed again.
    let mut g = primitives::grid(3, 3, 3.0);
    let r = rip(&mut g, &[5, 6], &[[5, 6]], true);
    assert_eq!(g.vertex_count(), 18);
    assert_eq!(r.selection.verts.len(), 2);
    assert_eq!(r.fill_faces.len(), 3);
    assert_eq!(boundary_edges(&g), 12);
    assert!(g.validate().is_ok());
    // Without fill the slit is open.
    let mut g = primitives::grid(3, 3, 3.0);
    rip(&mut g, &[5, 6], &[[5, 6]], false);
    assert_eq!(boundary_edges(&g), 12 + 6);
    // Rip and extend: a new vertex inserted on a boundary edge.
    let mut p = primitives::plane(2.0);
    let s = rip_edge(&mut p, &[0]);
    assert_eq!(p.vertex_count(), 5);
    assert_eq!(p.faces[0].len(), 5);
    assert_eq!(s.verts, vec![4]);
}

// ---------------------------------------------------------------------------
// Fills

#[test]
fn fill_closes_a_hole() {
    let mut c = primitives::cube(2.0);
    ops1::delete_faces(&mut c, &[1]);
    let lp = ops1::boundary_loops(&c).remove(0);
    let edges: Vec<[u32; 2]> = (0..lp.len()).map(|i| [lp[i], lp[(i + 1) % lp.len()]]).collect();
    let s = fill(&mut c, &edges, true);
    assert_eq!(s.faces.len(), 2);
    assert!(closed(&c));
    assert!((c.volume() - 8.0).abs() < 1e-4);
}

#[test]
fn grid_fill_square_loop() {
    // 12 edge loop around a 3x3 grid's border, as wire edges.
    let g = primitives::grid(3, 3, 3.0);
    let border = ops1::boundary_loops(&g).remove(0);
    let mut m = Mesh { positions: g.positions.clone(), faces: Vec::new(), loose_edges: Vec::new() };
    let n = border.len();
    let edges: Vec<[u32; 2]> = (0..n).map(|i| [border[i], border[(i + 1) % n]]).collect();
    m.loose_edges = edges.clone();
    let s = fill_grid(&mut m, &edges).unwrap();
    assert_eq!(s.faces.len(), 9);
    assert!(m.loose_edges.is_empty());
    m.remove_unused_vertices();
    assert_eq!(counts(&m), (16, 24, 9));
    // Interior points land on the grid.
    for p in &m.positions {
        let snap = |x: f32| ((x + 1.5) - (x + 1.5).round()).abs() < 1e-4;
        assert!(snap(p.x) && snap(p.y), "{p:?}");
    }
    let normals: Vec<f32> = (0..9).map(|f| m.face_normal(f).z).collect();
    assert!(normals.iter().all(|z| z.abs() > 0.99 && z.signum() == normals[0].signum()));
    // Odd loops are refused.
    let mut t = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)], vec![]);
    t.loose_edges = vec![[0, 1], [1, 2], [2, 0]];
    assert!(fill_grid(&mut t, &[[0, 1], [1, 2], [2, 0]]).is_err());
}

#[test]
fn face_split_by_wire_edges() {
    let mut p = primitives::plane(2.0);
    p.loose_edges.push([0, 3]);
    let s = face_split_by_edges(&mut p, &[[0, 3]]);
    assert_eq!(p.face_count(), 2);
    assert!(p.loose_edges.is_empty());
    assert_eq!(s.edges, vec![[0, 3]]);
    // Through a wire vertex in the middle.
    let mut p = primitives::plane(2.0);
    let c = p.add_vertex(Vec3::ZERO);
    p.loose_edges = vec![[0, c], [c, 3]];
    face_split_by_edges(&mut p, &[[0, c], [c, 3]]);
    assert_eq!(p.face_count(), 2);
    assert!(p.faces.iter().all(|f| f.len() == 4 && f.verts.contains(&c)));
    assert!((p.surface_area() - 4.0).abs() < 1e-5);
}

// ---------------------------------------------------------------------------
// Connect

#[test]
fn vert_connect_pairs() {
    let mut p = primitives::plane(2.0);
    let s = vert_connect(&mut p, &[0, 3]);
    assert_eq!(p.face_count(), 2);
    assert_eq!(s.edges, vec![[0, 3]]);
    assert!((p.surface_area() - 4.0).abs() < 1e-5);
    // Three of a hexagon's corners: three ears and a middle triangle.
    let pts: Vec<Vec3> = (0..6).map(|i| {
        let a = i as f32 * core::f32::consts::TAU / 6.0;
        v(a.cos(), a.sin(), 0.0)
    }).collect();
    let mut h = Mesh::from_faces(pts, vec![(0..6).collect()]);
    let area = h.surface_area();
    vert_connect(&mut h, &[0, 2, 4]);
    assert_eq!(h.face_count(), 4);
    assert!(h.faces.iter().all(|f| f.len() == 3));
    assert!((h.surface_area() - area).abs() < 1e-5);
}

#[test]
fn connect_path_across_faces() {
    let mut g = primitives::grid(2, 1, 2.0);
    // Opposite corners of the two-quad strip: the cut crosses edge 1-4.
    let s = vert_connect_path(&mut g, &[0, 5]);
    assert_eq!(g.vertex_count(), 7);
    assert_eq!(g.face_count(), 4);
    assert_eq!(s.edges.len(), 2);
    assert!(g.positions[6].approx_eq(Vec3::new(0.0, 0.0, 0.0), 1e-5));
    assert!((g.surface_area() - 4.0).abs() < 1e-5);
    assert!(g.validate().is_ok());
}

#[test]
fn split_concave_and_nonplanar() {
    let pts = [(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (1.0, 1.0), (1.0, 3.0), (0.0, 3.0)];
    let mut l = Mesh::from_faces(pts.iter().map(|&(x, y)| v(x, y, 0.0)).collect(), vec![(0..6).collect()]);
    let s = vert_connect_concave(&mut l, &[0]);
    assert!((2..=3).contains(&l.face_count()), "{}", l.face_count());
    assert_eq!(s.faces.len(), l.face_count());
    assert!((l.surface_area() - 4.0).abs() < 1e-5);
    for f in &l.faces {
        let k = f.len();
        let p = |i: usize| l.positions[f.verts[i % k] as usize];
        assert!((0..k).all(|i| (p(i + 1) - p(i)).cross(p(i + 2) - p(i + 1)).z >= -1e-6), "concave piece {:?}", f.verts);
    }
    let mut q = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(1.0, 1.0, 1.0), v(0.0, 1.0, 0.0)], vec![vec![0, 1, 2, 3]]);
    vert_connect_nonplanar(&mut q, &[0], 5f32.to_radians());
    assert_eq!(q.face_count(), 2);
    let mut flat = primitives::plane(2.0);
    vert_connect_nonplanar(&mut flat, &[0], 5f32.to_radians());
    assert_eq!(flat.face_count(), 1);
}

// ---------------------------------------------------------------------------
// Spin, screw, extrude

#[test]
fn spin_profile_into_closed_band() {
    let mut m = Mesh::from_faces(vec![v(1.0, 0.0, -1.0), v(1.0, 0.0, 0.0), v(1.0, 0.0, 1.0)], vec![]);
    m.loose_edges = vec![[0, 1], [1, 2]];
    spin(&mut m, &[], &[[0, 1], [1, 2]], &[], Vec3::ZERO, Vec3::Z, core::f32::consts::TAU, 8, 0.0);
    assert_eq!((m.vertex_count(), m.face_count()), (24, 16));
    assert!(m.loose_edges.is_empty());
    for f in 0..m.face_count() {
        let c = m.face_center(f);
        assert!(m.face_normal(f).dot(Vec3::new(c.x, c.y, 0.0)) > 0.0, "face {f} points inwards");
    }
    assert_eq!(boundary_edges(&m), 16);
    // A quarter turn in 3 steps.
    let mut m = Mesh::from_faces(vec![v(1.0, 0.0, 0.0), v(2.0, 0.0, 0.0)], vec![]);
    m.loose_edges = vec![[0, 1]];
    let s = spin(&mut m, &[], &[[0, 1]], &[], Vec3::ZERO, Vec3::Z, core::f32::consts::FRAC_PI_2, 3, 0.0);
    assert_eq!((m.vertex_count(), m.face_count()), (8, 3));
    assert!(s.verts.iter().all(|&x| m.positions[x as usize].x.abs() < 1e-5));
    // Spinning a face a full turn makes a closed ring.
    let mut q = Mesh::from_faces(vec![v(1.0, 0.0, -0.5), v(2.0, 0.0, -0.5), v(2.0, 0.0, 0.5), v(1.0, 0.0, 0.5)], vec![vec![0, 1, 2, 3]]);
    spin(&mut q, &[0], &[], &[], Vec3::ZERO, Vec3::Z, core::f32::consts::TAU, 8, 0.0);
    assert_eq!((q.vertex_count(), q.face_count()), (32, 32));
    assert!(closed(&q));
    assert!(q.volume() > 0.0);
}

#[test]
fn screw_merges_turns() {
    let mut m = Mesh::from_faces(vec![v(1.0, 0.0, 0.0), v(1.0, 0.0, 1.0)], vec![]);
    m.loose_edges = vec![[0, 1]];
    screw(&mut m, &[[0, 1]], Vec3::ZERO, Vec3::Z, 4, 2).unwrap();
    // The profile spans one pitch, so each turn's ends meet the next turn's.
    assert_eq!((m.vertex_count(), m.face_count()), (13, 8));
    let zmax = m.positions.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!((zmax - 3.0).abs() < 1e-4);
    assert!(screw(&mut m, &[], Vec3::ZERO, Vec3::Z, 4, 1).is_err());
}

#[test]
fn extrude_repeat_and_manifold() {
    let mut p = primitives::plane(2.0);
    let s = extrude_repeat(&mut p, &[0], &[], &[], Vec3::Z, 3);
    assert_eq!((p.vertex_count(), p.face_count()), (16, 13));
    assert!(s.verts.iter().all(|&x| (p.positions[x as usize].z - 3.0).abs() < 1e-6));
    // Edges only.
    let mut w = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0)], vec![]);
    w.loose_edges = vec![[0, 1]];
    extrude_repeat(&mut w, &[], &[[0, 1]], &[0, 1], Vec3::Z, 2);
    assert_eq!((w.vertex_count(), w.face_count()), (6, 2));
    // Manifold extrude of a cube's top: the sides grow instead of new walls.
    let mut c = primitives::cube(2.0);
    let top = c.faces.iter().position(|f| f.verts == vec![4, 5, 7, 6]).unwrap() as u32;
    let s = extrude_manifold(&mut c, &[top]);
    assert_eq!(counts(&c), (8, 12, 6));
    assert!(closed(&c));
    for &x in &s.verts {
        c.positions[x as usize].z += 1.0;
    }
    assert!((c.volume() - 12.0).abs() < 1e-4);
    // On a flat plane region the walls stay.
    let mut g = primitives::grid(3, 3, 3.0);
    extrude_manifold(&mut g, &[4]);
    assert_eq!(g.face_count(), 13);
}

// ---------------------------------------------------------------------------
// Symmetry, planar, hull, boolean

#[test]
fn symmetrize_mirrors_negative_half() {
    let mut c = primitives::cube(2.0);
    c.translate(Vec3::new(0.5, 0.0, 0.0));
    let af = all_faces(&c);
    let s = symmetrize(&mut c, &af, 0, true, 1e-4);
    assert!(closed(&c), "{:?}", counts(&c));
    assert_eq!(c.face_count(), 10);
    assert_eq!(s.faces.len(), 10);
    let b = c.bounds();
    assert!((b.min.x + 0.5).abs() < 1e-5 && (b.max.x - 0.5).abs() < 1e-5, "{b:?}");
    assert!((c.volume() - 4.0).abs() < 1e-4);
}

#[test]
fn symmetry_snap_pairs_and_centre() {
    let mut m = Mesh::from_faces(vec![v(-1.0, 0.0, 0.0), v(1.02, 0.01, 0.0), v(0.01, 1.0, 0.0)], vec![vec![0, 1, 2]]);
    symmetry_snap(&mut m, &[0, 1, 2], 0, true, 0.05, 0.5, true);
    let (a, b) = (m.positions[0], m.positions[1]);
    assert!(a.approx_eq(Vec3::new(-b.x, b.y, b.z), 1e-6));
    assert!((a.x + 1.01).abs() < 1e-5);
    assert_eq!(m.positions[2].x, 0.0);
}

#[test]
fn make_planar_flattens() {
    let mut q = Mesh::from_faces(vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(1.0, 1.0, 0.2), v(0.0, 1.0, 0.0)], vec![vec![0, 1, 2, 3]]);
    face_make_planar(&mut q, &[0], 1.0, 1);
    let n = q.face_normal(0);
    let c = q.face_center(0);
    assert!(q.positions.iter().all(|&p| (p - c).dot(n).abs() < 1e-5));
}

#[test]
fn convex_hull_of_cube_and_sphere() {
    let mut c = primitives::cube(2.0);
    let center = c.add_vertex(Vec3::ZERO);
    c.loose_edges.push([0, center]);
    let av = all_verts(&c);
    let af = all_faces(&c);
    let s = convex_hull(&mut c, &av, &af).unwrap();
    assert_eq!(counts(&c), (8, 12, 6));
    assert!(closed(&c));
    assert_eq!(s.faces.len(), 6);
    assert!((c.volume() - 8.0).abs() < 1e-4);
    let pts = primitives::ico_sphere(2, 1.0).positions;
    let tris = convex_hull_triangles(&pts).unwrap();
    assert_eq!(tris.len(), 320);
    assert!(convex_hull_triangles(&primitives::grid(2, 2, 1.0).positions).is_none());
}

#[test]
fn boolean_intersect_cuts_unselected() {
    let mut m = primitives::cube(2.0);
    let mut b = primitives::cube(2.0);
    b.translate(Vec3::new(1.0, 0.0, 0.0));
    let off = m.face_count() as u32;
    m.append(&b);
    let sel: Vec<u32> = (off..off + 6).collect();
    intersect_boolean(&mut m, &sel, BooleanOp::Difference).unwrap();
    assert!(closed(&m));
    assert!((m.volume() - 4.0).abs() < 1e-3, "{}", m.volume());
    assert!(intersect_boolean(&mut m, &[], BooleanOp::Union).is_err());
}

#[test]
fn merge_at_vertex_keeps_target_position() {
    let mut c = primitives::cube(2.0);
    let p1 = c.positions[1];
    let v = merge_at_vertex(&mut c, &[0, 1], 1).unwrap();
    assert_eq!(c.vertex_count(), 7);
    assert_eq!(c.positions[v as usize], p1);
}

#[test]
fn offset_edge_loops_adds_two_loops() {
    let mut c = primitives::cube(2.0);
    let new = ops1::loop_cut(&mut c, [0, 1], 1);
    let set: HashSet<u32> = new.iter().copied().collect();
    let lp: Vec<[u32; 2]> = c.edges().into_iter().filter(|e| set.contains(&e[0]) && set.contains(&e[1])).collect();
    let s = offset_edge_loops(&mut c, &lp, 0.25);
    assert_eq!(counts(&c), (20, 36, 18));
    assert!(closed(&c));
    assert_eq!(s.edges.len(), 8);
    assert!((c.volume() - 8.0).abs() < 1e-4);
}

#[test]
fn laplacian_smooth_flattens_a_bump() {
    let mut g = primitives::grid(4, 4, 4.0);
    g.positions[12].z = 1.0;
    let border: Vec<Vec3> = ops1::boundary_loops(&g)[0].iter().map(|&v| g.positions[v as usize]).collect();
    let av = all_verts(&g);
    smooth_laplacian(&mut g, &av, 1, 1.0, 5e-5, false);
    assert!(g.positions[12].z < 0.6, "{}", g.positions[12].z);
    let after: Vec<Vec3> = ops1::boundary_loops(&g)[0].iter().map(|&v| g.positions[v as usize]).collect();
    for (a, b) in border.iter().zip(&after) {
        assert!(a.approx_eq(*b, 1e-3));
    }
    // Volume preservation on a closed mesh.
    let mut s = primitives::ico_sphere(2, 1.0);
    let v0 = s.volume();
    let av = all_verts(&s);
    smooth_laplacian(&mut s, &av, 2, 1.0, 5e-5, true);
    assert!((s.volume() - v0).abs() < 1e-3 * v0);
}

// ---------------------------------------------------------------------------
// Selection queries

#[test]
fn selection_queries() {
    let g = primitives::grid(2, 2, 2.0);
    let nm = non_manifold(&g);
    assert_eq!(nm.edges.len(), 8);
    assert!(non_manifold(&primitives::cube(2.0)).edges.is_empty());
    // Two cubes touching at a corner: that vertex is non-manifold.
    let mut two = primitives::cube(2.0);
    let mut b = primitives::cube(2.0);
    b.translate(Vec3::splat(2.0));
    two.append(&b);
    two.weld(1e-5);
    let nm = non_manifold(&two);
    assert!(nm.edges.is_empty());
    assert_eq!(nm.verts.len(), 1);
    let c = primitives::cube(2.0);
    let map = mirror_vertex_map(&c, 0, 1e-4);
    assert_eq!(map[0], Some(1));
    assert_eq!(map[6], Some(7));
    let g = primitives::grid(3, 3, 3.0);
    let path = shortest_path_verts(&g, 0, 15).unwrap();
    assert_eq!(path.len(), 7);
    assert_eq!((path[0], path[6]), (0, 15));
    let fpath = shortest_path_faces(&g, 0, 8).unwrap();
    assert_eq!(fpath.len(), 5);
    let ring = edge_ring(&primitives::grid(3, 1, 3.0), [1, 5]);
    assert_eq!(ring.len(), 4);
    let ring = edge_ring(&primitives::cube(2.0), [0, 1]);
    assert_eq!(ring.len(), 4);
    // Two boxes stacked with a shared middle face: that face is interior.
    let mut stack = Mesh::new();
    for z in [0.0, 1.0, 2.0] {
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            stack.add_vertex(v(x, y, z));
        }
    }
    let side = |lo: u32, hi: u32| -> Vec<Vec<u32>> { (0..4).map(|i| vec![lo + i, lo + (i + 1) % 4, hi + (i + 1) % 4, hi + i]).collect() };
    let mut faces = vec![vec![3, 2, 1, 0], vec![4, 5, 6, 7], vec![8, 9, 10, 11]];
    faces.extend(side(0, 4));
    faces.extend(side(4, 8));
    let stack = Mesh { faces: faces.into_iter().map(Face::new).collect(), ..stack };
    assert_eq!(interior_faces(&stack), vec![1]);
}

