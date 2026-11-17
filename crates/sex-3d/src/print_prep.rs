// SPDX-License-Identifier: GPL-3.0-only
//! 3D-printing preparation, physical scaling, manifold validation/repair, and
//! seam-preserving QEM decimation.
//!
//! Units follow the 3D-printing convention where `1.0` mesh unit equals `1 mm`
//! (matching binary/ASCII STL slicers), so `1 cm³ = 1,000 mm³` and
//! `1 cm² = 100 mm²`.
use crate::math::{Mat4, Quat, Vec2, Vec3};
use crate::mesh::{edge_key, AttrDomain, Face, Mesh};
use crate::ops;
use crate::ops2::convex_hull_triangles;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Physical bounding box extents in millimetres (Z-up: X width, Y depth, Z height).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoundsMm {
    pub width: f32,
    pub depth: f32,
    pub height: f32,
}

impl BoundsMm {
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 && self.depth <= 0.0 && self.height <= 0.0
    }
}

/// Summary of 3D-printing metrics for resin/filament estimation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PrintMetrics {
    pub bounds_mm: BoundsMm,
    pub volume_cm3: f32,
    pub surface_area_cm2: f32,
    pub triangle_count: usize,
    /// Estimated photopolymer resin mass in grams (density ~1.12 g/cm³).
    pub est_resin_g: f32,
    /// Estimated solid-wall PLA filament mass in grams (density ~1.24 g/cm³).
    pub est_filament_g: f32,
}

/// Watertight and 2-manifold diagnostic report for a [`Mesh`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ManifoldReport {
    pub vertex_count: usize,
    pub face_count: usize,
    pub triangle_count: usize,
    pub edge_count: usize,
    /// Undirected edges incident to only 1 face.
    pub boundary_edges: usize,
    /// Undirected edges shared by more than 2 faces.
    pub non_manifold_edges: usize,
    /// Shared 2-face edges where adjacent faces traverse the edge in the same
    /// direction (flipped winding between neighbours).
    pub inconsistent_winding_edges: usize,
    /// Vertices with multiple disconnected face fans (pinch vertices).
    pub non_manifold_vertices: usize,
    /// Faces with invalid vertex counts, repeated vertices, or near-zero area.
    pub degenerate_faces: usize,
    /// True when every connected face shell has consistent outward winding.
    pub outward_winding: bool,
    /// True when the mesh is 2-manifold (allows open boundaries).
    pub is_manifold: bool,
    /// True when closed, 2-manifold, free of degenerate faces, and wound outward.
    pub is_watertight: bool,
}

// ---------------------------------------------------------------------------
// Real-world physical scaling (mm) & volume / surface area

/// Compute the physical bounding-box dimensions (`width`, `depth`, `height`) in mm.
pub fn mesh_bounds_mm(mesh: &Mesh) -> BoundsMm {
    let b = mesh.bounds();
    if b.is_empty() {
        return BoundsMm::default();
    }
    let s = b.size();
    BoundsMm {
        width: s.x.max(0.0),
        depth: s.y.max(0.0),
        height: s.z.max(0.0),
    }
}

/// Uniformly scale `mesh` so its Z-height equals `target_height_mm`.
/// Preserves the mesh origin (so a bed-aligned `min.z == 0.0` remains at `0.0`).
pub fn scale_to_height_mm(mesh: &mut Mesh, target_height_mm: f32) {
    if !target_height_mm.is_finite() || target_height_mm <= 0.0 {
        return;
    }
    let b = mesh.bounds();
    if b.is_empty() {
        return;
    }
    let h = b.size().z;
    if !h.is_finite() || h <= 1e-8 {
        return;
    }
    let s = target_height_mm / h;
    for p in &mut mesh.positions {
        *p *= s;
    }
    for k in &mut mesh.shape_keys {
        for p in &mut k.positions {
            *p *= s;
        }
    }
}

/// Signed volume in mm³ computed via signed tetrahedra around the mesh centroid
/// (minimizing floating-point cancellation for meshes far from the origin).
pub fn signed_volume_mm3(mesh: &Mesh) -> f32 {
    if mesh.positions.is_empty() || mesh.faces.is_empty() {
        return 0.0;
    }
    let b = mesh.bounds();
    let origin = if b.is_empty() {
        [0.0f64; 3]
    } else {
        let c = b.center();
        [c.x as f64, c.y as f64, c.z as f64]
    };
    let mut vol6 = 0.0f64;
    for t in mesh.triangulate() {
        let [a, b, c] = t.verts.map(|i| {
            let p = mesh.positions[i as usize];
            [p.x as f64 - origin[0], p.y as f64 - origin[1], p.z as f64 - origin[2]]
        });
        let bc = [
            b[1] * c[2] - b[2] * c[1],
            b[2] * c[0] - b[0] * c[2],
            b[0] * c[1] - b[1] * c[0],
        ];
        vol6 += a[0] * bc[0] + a[1] * bc[1] + a[2] * bc[2];
    }
    (vol6 / 6.0) as f32
}

/// Physical enclosed volume in cm³ (`|signed_volume_mm3| / 1000`).
pub fn volume_cm3(mesh: &Mesh) -> f32 {
    signed_volume_mm3(mesh).abs() / 1000.0
}

/// Physical total surface area in cm² (`area_mm2 / 100`).
pub fn surface_area_cm2(mesh: &Mesh) -> f32 {
    let mut area2 = 0.0f64;
    for t in mesh.triangulate() {
        let [a, b, c] = t.verts.map(|i| {
            let p = mesh.positions[i as usize];
            [p.x as f64, p.y as f64, p.z as f64]
        });
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cr = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        area2 += (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt();
    }
    (area2 * 0.5 / 100.0) as f32
}

/// Compute full 3D-printing metrics (dimensions, volume, area, material estimates).
pub fn print_metrics(mesh: &Mesh) -> PrintMetrics {
    let bounds_mm = mesh_bounds_mm(mesh);
    let volume_cm3 = volume_cm3(mesh);
    let surface_area_cm2 = surface_area_cm2(mesh);
    let triangle_count = mesh.triangulate().len();
    PrintMetrics {
        bounds_mm,
        volume_cm3,
        surface_area_cm2,
        triangle_count,
        est_resin_g: volume_cm3 * 1.12,
        est_filament_g: volume_cm3 * 1.24,
    }
}

// ---------------------------------------------------------------------------
// Auto-upright & print bed alignment

#[derive(Clone, Debug)]
struct HullCluster {
    normal: Vec3,
    hull_area: f32,
    contact_area: f32,
}

/// Area-weighted surface centroid (falling back to vertex mean).
fn mesh_centroid(mesh: &Mesh) -> Vec3 {
    if mesh.positions.is_empty() {
        return Vec3::ZERO;
    }
    let mut sum = [0.0f64; 3];
    let mut wsum = 0.0f64;
    for t in mesh.triangulate() {
        let [a, b, c] = t.verts.map(|i| mesh.positions[i as usize]);
        let area = ((b - a).cross(c - a).length() * 0.5) as f64;
        if area > 0.0 && area.is_finite() {
            let m = (a + b + c) / 3.0;
            sum[0] += m.x as f64 * area;
            sum[1] += m.y as f64 * area;
            sum[2] += m.z as f64 * area;
            wsum += area;
        }
    }
    if wsum > 1e-18 {
        return Vec3::new((sum[0] / wsum) as f32, (sum[1] / wsum) as f32, (sum[2] / wsum) as f32);
    }
    let n = mesh.positions.len() as f64;
    let mut vsum = [0.0f64; 3];
    for &p in &mesh.positions {
        vsum[0] += p.x as f64;
        vsum[1] += p.y as f64;
        vsum[2] += p.z as f64;
    }
    Vec3::new((vsum[0] / n) as f32, (vsum[1] / n) as f32, (vsum[2] / n) as f32)
}

/// Surface-area-weighted 3x3 covariance PCA, returning eigenvalues (desc) and right-handed basis.
fn mesh_pca(mesh: &Mesh, center: Vec3) -> ([f64; 3], [Vec3; 3]) {
    let mut cov = [[0.0f64; 3]; 3];
    let mut total_w = 0.0f64;
    let mu = [center.x as f64, center.y as f64, center.z as f64];

    for t in mesh.triangulate() {
        let [pa, pb, pc] = t.verts.map(|i| {
            let p = mesh.positions[i as usize];
            [p.x as f64 - mu[0], p.y as f64 - mu[1], p.z as f64 - mu[2]]
        });
        let ab = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let ac = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let cr = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];
        let area = 0.5 * (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt();
        if area <= 1e-18 || !area.is_finite() {
            continue;
        }
        let pm = [
            (pa[0] + pb[0] + pc[0]) / 3.0,
            (pa[1] + pb[1] + pc[1]) / 3.0,
            (pa[2] + pb[2] + pc[2]) / 3.0,
        ];
        let w = area / 12.0;
        for i in 0..3 {
            for j in 0..3 {
                cov[i][j] += w * (pa[i] * pa[j] + pb[i] * pb[j] + pc[i] * pc[j] + 9.0 * pm[i] * pm[j]);
            }
        }
        total_w += area;
    }

    if total_w <= 1e-18 {
        for &p in &mesh.positions {
            let q = [p.x as f64 - mu[0], p.y as f64 - mu[1], p.z as f64 - mu[2]];
            for i in 0..3 {
                for j in 0..3 {
                    cov[i][j] += q[i] * q[j];
                }
            }
            total_w += 1.0;
        }
    }
    if total_w > 0.0 {
        for r in &mut cov {
            for v in r.iter_mut() {
                *v /= total_w;
            }
        }
    }

    // Jacobi eigensolver on symmetric 3x3 matrix.
    let mut a = cov;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..24 {
        let mut best = 0.0;
        let (mut p, mut q) = (0, 1);
        for i in 0..3 {
            for j in (i + 1)..3 {
                let val = a[i][j].abs();
                if val > best {
                    best = val;
                    p = i;
                    q = j;
                }
            }
        }
        if best <= 1e-14 {
            break;
        }
        let app = a[p][p];
        let aqq = a[q][q];
        let apq = a[p][q];
        let tau = (aqq - app) / (2.0 * apq);
        let t = if tau >= 0.0 {
            1.0 / (tau + (1.0 + tau * tau).sqrt())
        } else {
            -1.0 / (-tau + (1.0 + tau * tau).sqrt())
        };
        let c = 1.0 / (1.0 + t * t).sqrt();
        let s = t * c;

        a[p][p] = app - t * apq;
        a[q][q] = aqq + t * apq;
        a[p][q] = 0.0;
        a[q][p] = 0.0;
        for r in 0..3 {
            if r == p || r == q {
                continue;
            }
            let arp = a[r][p];
            let arq = a[r][q];
            a[r][p] = c * arp - s * arq;
            a[p][r] = a[r][p];
            a[r][q] = s * arp + c * arq;
            a[q][r] = a[r][q];
        }
        for row in &mut v {
            let vrp = row[p];
            let vrq = row[q];
            row[p] = c * vrp - s * vrq;
            row[q] = s * vrp + c * vrq;
        }
    }

    let mut pairs = [
        (a[0][0].max(0.0), Vec3::new(v[0][0] as f32, v[1][0] as f32, v[2][0] as f32).normalize()),
        (a[1][1].max(0.0), Vec3::new(v[0][1] as f32, v[1][1] as f32, v[2][1] as f32).normalize()),
        (a[2][2].max(0.0), Vec3::new(v[0][2] as f32, v[1][2] as f32, v[2][2] as f32).normalize()),
    ];
    pairs.sort_by(|x, y| y.0.total_cmp(&x.0));
    let e0 = if pairs[0].1 == Vec3::ZERO { Vec3::X } else { pairs[0].1 };
    let e1_raw = if pairs[1].1 == Vec3::ZERO { e0.any_orthonormal() } else { pairs[1].1 };
    let e1 = (e1_raw - e0 * e1_raw.dot(e0)).normalize();
    let e2 = e0.cross(e1).normalize();
    ([pairs[0].0, pairs[1].0, pairs[2].0], [e0, e1, e2])
}

/// Group convex-hull triangles into coplanar support planes and measure both hull
/// area and actual mesh face contact area on each extreme plane.
fn hull_support_clusters(mesh: &Mesh) -> Vec<HullCluster> {
    let bounds = mesh.bounds();
    if bounds.is_empty() || mesh.positions.len() < 4 {
        return Vec::new();
    }
    let scale = bounds.size().length().max(1e-4);
    let Some(hull_tris) = convex_hull_triangles(&mesh.positions) else {
        return Vec::new();
    };
    let tris = mesh.triangulate();
    let tri_info: Vec<([Vec3; 3], Vec3, f32)> = tris
        .iter()
        .filter_map(|t| {
            let pts = t.verts.map(|i| mesh.positions[i as usize]);
            let cr = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
            let l = cr.length();
            (l > 1e-12 * scale * scale).then(|| (pts, cr / l, l * 0.5))
        })
        .collect();

    struct RawCluster {
        n_sum: Vec3,
        area_sum: f32,
        d_weighted: f32,
    }
    let mut raw: Vec<RawCluster> = Vec::new();
    let plane_tol = 2e-3 * scale;

    for t in hull_tris {
        let pts = t.map(|i| mesh.positions[i]);
        let cr = (pts[1] - pts[0]).cross(pts[2] - pts[0]);
        let l = cr.length();
        if l <= 1e-12 * scale * scale {
            continue;
        }
        let area = l * 0.5;
        let n = cr / l;
        let d = n.dot(pts[0]);
        let mut matched = None;
        for (ci, c) in raw.iter().enumerate() {
            let cn = c.n_sum.normalize();
            let cd = c.d_weighted / c.area_sum;
            if cn.dot(n) > 0.997 && (cd - d).abs() <= plane_tol {
                matched = Some(ci);
                break;
            }
        }
        match matched {
            Some(ci) => {
                raw[ci].n_sum += n * area;
                raw[ci].area_sum += area;
                raw[ci].d_weighted += d * area;
            }
            None => raw.push(RawCluster {
                n_sum: n * area,
                area_sum: area,
                d_weighted: d * area,
            }),
        }
    }

    let mut out = Vec::with_capacity(raw.len());
    for c in raw {
        let n = c.n_sum.normalize();
        if n == Vec3::ZERO {
            continue;
        }
        let d = c.d_weighted / c.area_sum;
        let mut contact = 0.0f32;
        for &(pts, tn, ta) in &tri_info {
            if tn.dot(n) > 0.98 && pts.iter().all(|&p| (n.dot(p) - d).abs() <= plane_tol) {
                contact += ta;
            }
        }
        out.push(HullCluster {
            normal: n,
            hull_area: c.area_sum,
            contact_area: contact,
        });
    }
    out
}

/// Shortest-arc quaternion mapping unit vector `from` to `to`.
fn quat_from_to(from: Vec3, to: Vec3) -> Quat {
    let f = from.normalize();
    let t = to.normalize();
    if f == Vec3::ZERO || t == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let d = f.dot(t);
    if d >= 1.0 - 1e-6 {
        return Quat::IDENTITY;
    }
    if d <= -1.0 + 1e-6 {
        // Flip 180 degrees around X (or any orthonormal if parallel to X).
        let axis = if f.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
        let ortho = f.cross(axis).normalize();
        return Quat::from_axis_angle(ortho, core::f32::consts::PI);
    }
    let axis = f.cross(t).normalize();
    Quat::from_axis_angle(axis, d.clamp(-1.0, 1.0).acos())
}

fn rotate_mesh_around(mesh: &mut Mesh, center: Vec3, q: Quat) {
    if (q.w.abs() - 1.0).abs() < 1e-7 {
        return;
    }
    let m = Mat4::translation(center) * Mat4::from_trs(Vec3::ZERO, q, Vec3::ONE) * Mat4::translation(-center);
    mesh.transform(&m);
}

/// Orient `mesh` upright along `+Z` using surface-area-weighted PCA and convex-hull
/// support plane alignment.
pub fn orient_upright(mesh: &mut Mesh) {
    if mesh.positions.len() < 3 {
        return;
    }
    let bounds = mesh.bounds();
    if bounds.is_empty() || bounds.size().length() <= 1e-8 {
        return;
    }
    let center = mesh_centroid(mesh);
    let (evals, axes) = mesh_pca(mesh, center);
    let clusters = hull_support_clusters(mesh);
    let total_hull: f32 = clusters.iter().map(|c| c.hull_area).sum();

    let mut up = Vec3::Z;
    let elongated = evals[0] > 1.35 * evals[1].max(1e-12);

    if elongated {
        let e0 = axes[0];
        let mut pos_area = 0.0f32;
        let mut neg_area = 0.0f32;
        let mut pos_n = Vec3::ZERO;
        let mut neg_n = Vec3::ZERO;
        for c in &clusters {
            let dp = c.normal.dot(e0);
            if dp > 0.94 {
                pos_area += c.hull_area;
                pos_n += c.normal * c.hull_area;
            } else if dp < -0.94 {
                neg_area += c.hull_area;
                neg_n += c.normal * c.hull_area;
            }
        }
        // Choose sign so the heavier/wider support base points to -Z.
        if pos_area > neg_area * 1.25 && pos_area > 0.01 * total_hull {
            up = -e0;
            if pos_n != Vec3::ZERO && pos_n.normalize().dot(e0) > 0.96 {
                up = -pos_n.normalize();
            }
        } else if neg_area > pos_area * 1.25 && neg_area > 0.01 * total_hull {
            up = e0;
            if neg_n != Vec3::ZERO && neg_n.normalize().dot(-e0) > 0.96 {
                up = -neg_n.normalize();
            }
        } else {
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            for &p in &mesh.positions {
                let pr = p.dot(e0);
                lo = lo.min(pr);
                hi = hi.max(pr);
            }
            let mid = 0.5 * (lo + hi);
            let cen = center.dot(e0);
            let span = (hi - lo).max(1e-6);
            up = if (mid - cen).abs() > 0.02 * span {
                if mid >= cen { e0 } else { -e0 }
            } else if e0.z >= 0.0 {
                e0
            } else {
                -e0
            };
            // Snap to flat support cap aligned with -up if present.
            if let Some(cap) = clusters
                .iter()
                .filter(|c| c.normal.dot(-up) > 0.96 && c.hull_area > 0.02 * total_hull)
                .max_by(|a, b| a.hull_area.total_cmp(&b.hull_area))
            {
                up = -cap.normal;
            }
        }
    } else if !clusters.is_empty() {
        let max_area = clusters.iter().map(|c| c.hull_area).fold(0.0f32, f32::max);
        if max_area > 0.05 * total_hull {
            // Among near-tied largest flat faces (e.g. rotated cube), pick closest to -Z.
            if let Some(best) = clusters
                .iter()
                .filter(|c| c.hull_area >= max_area * 0.92)
                .max_by(|a, b| a.normal.dot(-Vec3::Z).total_cmp(&b.normal.dot(-Vec3::Z)))
            {
                up = -best.normal;
            }
        } else if evals[2] < 0.65 * evals[1].max(1e-12) {
            up = if axes[2].z >= 0.0 { axes[2] } else { -axes[2] };
        }
    } else if evals[2] < 0.65 * evals[1].max(1e-12) {
        up = if axes[2].z >= 0.0 { axes[2] } else { -axes[2] };
    }

    let q = quat_from_to(up, Vec3::Z);
    rotate_mesh_around(mesh, center, q);
}

/// Drop `mesh` onto the `Z = 0` print bed while orienting onto the flat
/// convex-hull support plane that maximizes bed contact area.
pub fn place_on_bed(mesh: &mut Mesh) {
    if mesh.positions.is_empty() {
        return;
    }
    let bounds = mesh.bounds();
    if bounds.is_empty() {
        return;
    }
    let center = mesh_centroid(mesh);
    let total_area = mesh.surface_area();
    let clusters = hull_support_clusters(mesh);

    if !clusters.is_empty() && total_area > 0.0 {
        let max_contact = clusters.iter().map(|c| c.contact_area).fold(0.0f32, f32::max);
        let max_hull = clusters.iter().map(|c| c.hull_area).fold(0.0f32, f32::max);
        let use_contact = max_contact > 0.015 * total_area;
        let threshold = if use_contact { max_contact * 0.95 } else { max_hull * 0.95 };

        // Check whether a clear planar support exists (avoid tilting smooth spheres).
        if use_contact || max_hull > 0.03 * total_area {
            let best = clusters
                .iter()
                .filter(|c| {
                    let score = if use_contact { c.contact_area } else { c.hull_area };
                    score >= threshold
                })
                // Tie-break by stability (centroid closer to face centroid along plane) & minimal tilt from -Z.
                .max_by(|a, b| {
                    let sa = if use_contact { a.contact_area } else { a.hull_area };
                    let sb = if use_contact { b.contact_area } else { b.hull_area };
                    let da = (a.normal.dot(-Vec3::Z), sa);
                    let db = (b.normal.dot(-Vec3::Z), sb);
                    da.0.total_cmp(&db.0).then_with(|| da.1.total_cmp(&db.1))
                });
            if let Some(c) = best {
                // Only rotate if it improves flat contact meaningfully or differs from -Z.
                let cur_bottom_contact = clusters
                    .iter()
                    .filter(|cl| cl.normal.dot(-Vec3::Z) > 0.998)
                    .map(|cl| cl.contact_area)
                    .fold(0.0f32, f32::max);
                if c.contact_area > cur_bottom_contact * 1.02 + 1e-5 || cur_bottom_contact <= 1e-6 * total_area {
                    let q = quat_from_to(c.normal, -Vec3::Z);
                    rotate_mesh_around(mesh, center, q);
                }
            }
        }
    } else if !mesh.faces.is_empty() && bounds.size().z < 1e-5 * bounds.size().length().max(1.0) {
        // Planar mesh: align normal closest to +Z.
        let n = mesh.face_normal(0);
        if n != Vec3::ZERO {
            let target = if n.z >= 0.0 { Vec3::Z } else { -Vec3::Z };
            rotate_mesh_around(mesh, center, quat_from_to(n, target));
        }
    }

    let min_z = mesh.positions.iter().map(|p| p.z).fold(f32::INFINITY, f32::min);
    if min_z.is_finite() {
        mesh.translate(Vec3::new(0.0, 0.0, -min_z));
    }
}

// ---------------------------------------------------------------------------
// Manifold inspection & auto-repair

fn is_valid_face_geometry(mesh: &Mesh, fi: usize, area_eps: f32) -> bool {
    let f = &mesh.faces[fi];
    let nv = mesh.positions.len();
    if f.verts.len() < 3 || f.verts.iter().any(|&v| v as usize >= nv) {
        return false;
    }
    let mut uniq = f.verts.clone();
    uniq.sort_unstable();
    uniq.dedup();
    if uniq.len() < 3 {
        return false;
    }
    let area = mesh.face_area(fi);
    area.is_finite() && area > area_eps
}

/// Inspect `mesh` for boundary edges, non-manifold edges/vertices, degenerate
/// faces, and consistent outward winding.
pub fn inspect_manifold(mesh: &Mesh) -> ManifoldReport {
    let nv = mesh.positions.len();
    let nf = mesh.faces.len();
    if nf == 0 {
        return ManifoldReport {
            vertex_count: nv,
            ..Default::default()
        };
    }

    let b = mesh.bounds();
    let scale = if b.is_empty() { 1.0 } else { b.size().length().max(1e-4) };
    let area_eps = 1e-12 * scale * scale;

    let mut degenerate_faces = 0usize;
    let mut edge_dirs: HashMap<(u32, u32), Vec<(usize, bool)>> = HashMap::new();
    let mut vert_faces: Vec<Vec<usize>> = vec![Vec::new(); nv];

    for (fi, f) in mesh.faces.iter().enumerate() {
        if !is_valid_face_geometry(mesh, fi, area_eps) {
            degenerate_faces += 1;
        }
        for &v in &f.verts {
            if (v as usize) < nv {
                vert_faces[v as usize].push(fi);
            }
        }
        for (a, b) in f.edges() {
            if a != b && (a as usize) < nv && (b as usize) < nv {
                edge_dirs.entry(edge_key(a, b)).or_default().push((fi, a < b));
            }
        }
    }

    let mut boundary_edges = 0usize;
    let mut non_manifold_edges = 0usize;
    let mut inconsistent_winding_edges = 0usize;

    // Adjacency between faces across 2-face edges for shell component checks.
    let mut adj: Vec<Vec<(usize, bool)>> = vec![Vec::new(); nf];
    for list in edge_dirs.values() {
        match list.len() {
            1 => boundary_edges += 1,
            2 => {
                let (f0, d0) = list[0];
                let (f1, d1) = list[1];
                let consistent = d0 != d1;
                if !consistent {
                    inconsistent_winding_edges += 1;
                }
                adj[f0].push((f1, consistent));
                adj[f1].push((f0, consistent));
            }
            _ => non_manifold_edges += 1,
        }
    }

    // Pinch-vertex check: vertices used by faces whose incident face graph
    // around that vertex has more than 1 connected component.
    let mut edge_faces_by_v: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (&ek, list) in &edge_dirs {
        edge_faces_by_v.insert(ek, list.iter().map(|x| x.0).collect());
    }
    let mut non_manifold_vertices = 0usize;
    for (vi, fset) in vert_faces.iter().enumerate() {
        if fset.len() <= 1 {
            continue;
        }
        let v = vi as u32;
        let local_set: HashSet<usize> = fset.iter().copied().collect();
        let mut visited: HashSet<usize> = HashSet::new();
        let mut stack = vec![fset[0]];
        visited.insert(fset[0]);
        while let Some(f) = stack.pop() {
            for (a, b) in mesh.faces[f].edges() {
                if a == v || b == v {
                    if let Some(nbrs) = edge_faces_by_v.get(&edge_key(a, b)) {
                        for &g in nbrs {
                            if local_set.contains(&g) && visited.insert(g) {
                                stack.push(g);
                            }
                        }
                    }
                }
            }
        }
        if visited.len() != local_set.len() {
            non_manifold_vertices += 1;
        }
    }

    // Outward winding check per connected component.
    let mut outward_winding = inconsistent_winding_edges == 0 && non_manifold_edges == 0;
    if outward_winding {
        let mut seen = vec![false; nf];
        for seed in 0..nf {
            if seen[seed] {
                continue;
            }
            seen[seed] = true;
            let mut stack = vec![seed];
            let mut comp = Vec::new();
            while let Some(f) = stack.pop() {
                comp.push(f);
                for &(g, _) in &adj[f] {
                    if !seen[g] {
                        seen[g] = true;
                        stack.push(g);
                    }
                }
            }
            let mut cen = Vec3::ZERO;
            let mut cnt = 0usize;
            for &f in &comp {
                for &v in &mesh.faces[f].verts {
                    if (v as usize) < nv {
                        cen += mesh.positions[v as usize];
                        cnt += 1;
                    }
                }
            }
            if cnt > 0 {
                cen = cen / cnt as f32;
                let mut signed_sum = 0.0f64;
                for &f in &comp {
                    let area = mesh.face_area(f) as f64;
                    let n = mesh.face_normal(f);
                    let fc = mesh.face_center(f) - cen;
                    signed_sum += area * (n.dot(fc) as f64);
                }
                if signed_sum <= 0.0 {
                    outward_winding = false;
                    break;
                }
            }
        }
    }

    let is_manifold = non_manifold_edges == 0
        && non_manifold_vertices == 0
        && inconsistent_winding_edges == 0
        && degenerate_faces == 0;
    let is_watertight = is_manifold && boundary_edges == 0 && outward_winding;

    ManifoldReport {
        vertex_count: nv,
        face_count: nf,
        triangle_count: mesh.triangulate().len(),
        edge_count: edge_dirs.len(),
        boundary_edges,
        non_manifold_edges,
        inconsistent_winding_edges,
        non_manifold_vertices,
        degenerate_faces,
        outward_winding,
        is_manifold,
        is_watertight,
    }
}

/// Remove degenerate faces (`< 3` unique verts, out-of-range indices, or zero area)
/// and exact/back-to-back duplicate faces.
fn cleanup_faces(mesh: &mut Mesh, area_eps: f32) {
    let nv = mesh.positions.len();
    let mut clean_faces = Vec::with_capacity(mesh.faces.len());
    for mut f in std::mem::take(&mut mesh.faces) {
        if f.verts.iter().any(|&v| v as usize >= nv) {
            continue;
        }
        let has_uv = f.uvs.len() == f.verts.len();
        let mut verts = Vec::with_capacity(f.verts.len());
        let mut uvs = Vec::new();
        for (i, &v) in f.verts.iter().enumerate() {
            if verts.last() != Some(&v) {
                verts.push(v);
                if has_uv {
                    uvs.push(f.uvs[i]);
                }
            }
        }
        while verts.len() > 1 && verts.first() == verts.last() {
            verts.pop();
            uvs.pop();
        }
        if verts.len() < 3 {
            continue;
        }
        f.verts = verts;
        f.uvs = uvs;
        clean_faces.push(f);
    }
    mesh.faces = clean_faces;

    let mut keep = vec![true; mesh.faces.len()];
    for fi in 0..mesh.faces.len() {
        if !is_valid_face_geometry(mesh, fi, area_eps) {
            keep[fi] = false;
        }
    }
    // Remove duplicate faces sharing the exact same vertex set.
    let mut by_set: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        if !keep[fi] {
            continue;
        }
        let mut k = f.verts.clone();
        k.sort_unstable();
        by_set.entry(k).or_default().push(fi);
    }
    for list in by_set.values() {
        if list.len() > 1 {
            // Keep first if odd number (duplicate exterior copy), drop pairs of back-to-back.
            if list.len() % 2 == 0 {
                for &fi in list {
                    keep[fi] = false;
                }
            } else {
                for &fi in &list[1..] {
                    keep[fi] = false;
                }
            }
        }
    }
    let mut idx = 0;
    mesh.faces.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

/// Close simple boundary loops using ear-clipped triangles that do not introduce
/// non-manifold edges.
fn close_simple_boundary_loops(mesh: &mut Mesh) {
    let loops = ops::boundary_loops(mesh);
    if loops.is_empty() {
        return;
    }
    let mut edge_count: HashMap<(u32, u32), usize> = HashMap::new();
    for f in &mesh.faces {
        for (a, b) in f.edges() {
            *edge_count.entry(edge_key(a, b)).or_default() += 1;
        }
    }
    let has_uvs = mesh.has_uvs();
    for lp in loops {
        if lp.len() < 3 {
            continue;
        }
        // Reversed boundary loop closes the hole with outward-matching winding.
        let rev: Vec<u32> = lp.into_iter().rev().collect();
        let tmp = Mesh {
            positions: mesh.positions.clone(),
            faces: vec![Face::new(rev)],
            ..Default::default()
        };
        let tris = tmp.triangulate();
        for t in tris {
            let [a, b, c] = t.verts;
            if a == b || b == c || a == c {
                continue;
            }
            let area = (mesh.positions[b as usize] - mesh.positions[a as usize])
                .cross(mesh.positions[c as usize] - mesh.positions[a as usize])
                .length()
                * 0.5;
            if area <= 1e-12 {
                continue;
            }
            let es = [edge_key(a, b), edge_key(b, c), edge_key(c, a)];
            if es.iter().any(|e| edge_count.get(e).copied().unwrap_or(0) >= 2) {
                continue;
            }
            for e in es {
                *edge_count.entry(e).or_default() += 1;
            }
            mesh.faces.push(Face {
                verts: vec![a, b, c],
                uvs: if has_uvs { vec![Vec2::ZERO; 3] } else { Vec::new() },
                ..Default::default()
            });
        }
    }
    if has_uvs {
        ops::fill_missing_uvs(mesh);
    }
}

/// Repair a mesh for 3D printing: welds near-coincident vertices, removes
/// degenerate triangles/faces, makes face normals consistent, closes simple
/// boundary loops, and recalculates outward winding.
pub fn repair_for_printing(mesh: &mut Mesh) {
    if mesh.positions.is_empty() || mesh.faces.is_empty() {
        return;
    }
    // Replace any non-finite positions cleanly before bounds calculations.
    for p in &mut mesh.positions {
        if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
            *p = Vec3::ZERO;
        }
    }
    let b = mesh.bounds();
    let scale = if b.is_empty() { 1.0 } else { b.size().length().max(1e-4) };
    let area_eps = 1e-12 * scale * scale;
    let weld_eps = 5e-5 * scale;

    cleanup_faces(mesh, area_eps);
    mesh.weld(weld_eps);
    cleanup_faces(mesh, area_eps);
    mesh.remove_unused_vertices();

    // Fix normals before filling loops so boundary_loops sees consistent edges.
    ops::recalc_normals_outside(mesh);
    close_simple_boundary_loops(mesh);
    ops::recalc_normals_outside(mesh);

    cleanup_faces(mesh, area_eps);
    mesh.remove_unused_vertices();
    mesh.prune_flags();
    mesh.sanitize_attributes();
}

// ---------------------------------------------------------------------------
// Boundary & UV-seam-preserving QEM Decimator

type Q = [f64; 10];
type P = [f64; 3];

const QEM_BOUNDARY_WEIGHT: f64 = 100.0;

fn plane_q(n: P, d: f64, w: f64) -> Q {
    let [a, b, c] = n;
    [a * a, a * b, a * c, a * d, b * b, b * c, b * d, c * c, c * d, d * d].map(|x| x * w)
}

fn add_q(a: &mut Q, b: &Q) {
    for i in 0..10 {
        a[i] += b[i];
    }
}

fn eval_q(q: &Q, p: P) -> f64 {
    let [x, y, z] = p;
    q[0] * x * x
        + 2.0 * q[1] * x * y
        + 2.0 * q[2] * x * z
        + 2.0 * q[3] * x
        + q[4] * y * y
        + 2.0 * q[5] * y * z
        + 2.0 * q[6] * y
        + q[7] * z * z
        + 2.0 * q[8] * z
        + q[9]
}

fn optimal_q(q: &Q) -> Option<P> {
    let m = [[q[0], q[1], q[2]], [q[1], q[4], q[5]], [q[2], q[5], q[7]]];
    let r = [-q[3], -q[6], -q[8]];
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    let tr = q[0] + q[4] + q[7];
    if !(d.abs() > 1e-7 * tr * tr * tr) {
        return None;
    }
    let mut out = [0.0; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let mut mi = m;
        for row in 0..3 {
            mi[row][i] = r[row];
        }
        *o = det(&mi) / d;
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

fn p_sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn p_cross(a: P, b: P) -> P {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn p_dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn p_len(a: P) -> f64 {
    p_dot(a, a).sqrt()
}
fn p_scale(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s, a[2] * s]
}

#[derive(PartialEq)]
struct HeapEntry {
    cost: f64,
    a: u32,
    b: u32,
    stamp: (u32, u32),
}
impl Eq for HeapEntry {}
impl Ord for HeapEntry {
    fn cmp(&self, o: &Self) -> Ordering {
        o.cost.total_cmp(&self.cost).then_with(|| (o.a, o.b).cmp(&(self.a, self.b)))
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

struct QemState {
    pos: Vec<P>,
    tv: Vec<[u32; 3]>,
    tuv: Vec<[Vec2; 3]>,
    has_uv: Vec<bool>,
    tmat: Vec<u16>,
    tsmooth: Vec<bool>,
    alive: Vec<bool>,
    vt: Vec<Vec<u32>>,
    q: Vec<Q>,
    boundary: Vec<bool>,
    seam_vert: Vec<bool>,
    seam_edges: HashSet<(u32, u32)>,
    boundary_edges: HashSet<(u32, u32)>,
    locked: Vec<bool>,
    version: Vec<u32>,
    vdead: Vec<bool>,
    merged: Vec<u32>,
}

impl QemState {
    fn neighbours(&self, v: u32) -> Vec<u32> {
        let mut out: Vec<u32> = self.vt[v as usize]
            .iter()
            .flat_map(|&t| self.tv[t as usize])
            .filter(|&u| u != v)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    fn is_feature_vert(&self, v: u32) -> bool {
        self.boundary[v as usize] || self.seam_vert[v as usize]
    }

    fn is_feature_edge(&self, a: u32, b: u32) -> bool {
        let k = edge_key(a, b);
        self.boundary_edges.contains(&k) || self.seam_edges.contains(&k)
    }

    fn plan(&self, a: u32, b: u32) -> (P, f64) {
        let (ai, bi) = (a as usize, b as usize);
        let mut q = self.q[ai];
        add_q(&mut q, &self.q[bi]);
        let (pa, pb) = (self.pos[ai], self.pos[bi]);
        let mid = p_scale([pa[0] + pb[0], pa[1] + pb[1], pa[2] + pb[2]], 0.5);
        let fa = self.is_feature_vert(a);
        let fb = self.is_feature_vert(b);
        let p = match (fa, fb) {
            (true, false) => pa,
            (false, true) => pb,
            (true, true) => {
                if eval_q(&q, pa) <= eval_q(&q, pb) { pa } else { pb }
            }
            (false, false) => {
                let mut cands = vec![pa, pb, mid];
                if let Some(o) = optimal_q(&q) {
                    let l = p_len(p_sub(pa, pb));
                    if p_len(p_sub(o, mid)) <= 2.0 * l + 1e-12 {
                        cands.insert(0, o);
                    }
                }
                cands
                    .into_iter()
                    .min_by(|x, y| eval_q(&q, *x).total_cmp(&eval_q(&q, *y)))
                    .unwrap()
            }
        };
        (p, eval_q(&q, p).max(0.0))
    }

    fn push(&self, heap: &mut BinaryHeap<HeapEntry>, a: u32, b: u32) {
        if self.locked[a as usize] || self.locked[b as usize] {
            return;
        }
        if self.is_feature_vert(a) && self.is_feature_vert(b) && !self.is_feature_edge(a, b) {
            return;
        }
        let (_, cost) = self.plan(a, b);
        heap.push(HeapEntry {
            cost,
            a,
            b,
            stamp: (self.version[a as usize], self.version[b as usize]),
        });
    }

    fn collapse(&mut self, a: u32, b: u32, p: P) -> Option<usize> {
        if self.locked[a as usize] || self.locked[b as usize] {
            return None;
        }
        let shared: Vec<u32> = self.vt[a as usize]
            .iter()
            .copied()
            .filter(|&t| self.tv[t as usize].contains(&b))
            .collect();
        if shared.is_empty() || shared.len() > 2 {
            return None;
        }
        let fa = self.is_feature_vert(a);
        let fb = self.is_feature_vert(b);
        if fa && fb && !self.is_feature_edge(a, b) {
            return None;
        }
        if shared.len() == 2 && self.boundary[a as usize] && self.boundary[b as usize] {
            return None;
        }
        // Link condition: common neighbours must be exactly the opposite apexes of `shared`.
        let (na, nb) = (self.neighbours(a), self.neighbours(b));
        let common = na.iter().filter(|v| nb.binary_search(v).is_ok()).count();
        if common != shared.len() {
            return None;
        }
        // Reject triangle normal flips or slivers.
        for &v in &[a, b] {
            for &t in &self.vt[v as usize] {
                if shared.contains(&t) {
                    continue;
                }
                let tri = self.tv[t as usize];
                let pts = tri.map(|x| self.pos[x as usize]);
                let n0 = p_cross(p_sub(pts[1], pts[0]), p_sub(pts[2], pts[0]));
                let moved = tri.map(|x| if x == v { p } else { self.pos[x as usize] });
                let n1 = p_cross(p_sub(moved[1], moved[0]), p_sub(moved[2], moved[0]));
                let (l0, l1) = (p_len(n0), p_len(n1));
                if l1 <= 1e-12 * (l0 + 1e-30) || p_dot(n0, n1) <= 0.2 * l0 * l1 {
                    return None;
                }
            }
        }

        // Compute UV interpolation along edge (a -> b) for each side of the shared triangles.
        let ab = p_sub(self.pos[b as usize], self.pos[a as usize]);
        let l2 = p_dot(ab, ab);
        let t_param = if l2 > 1e-24 {
            (p_dot(p_sub(p, self.pos[a as usize]), ab) / l2).clamp(0.0, 1.0) as f32
        } else {
            0.0
        };
        let mut shared_uvs: Vec<(Vec2, Vec2, Vec2)> = Vec::with_capacity(shared.len());
        for &st in &shared {
            if self.has_uv[st as usize] {
                let tv = self.tv[st as usize];
                let ia = tv.iter().position(|&v| v == a).unwrap();
                let ib = tv.iter().position(|&v| v == b).unwrap();
                let uva = self.tuv[st as usize][ia];
                let uvb = self.tuv[st as usize][ib];
                shared_uvs.push((uva, uvb, uva.lerp(uvb, t_param)));
            }
        }

        // Verify and precompute UV updates for incident triangles before mutating.
        let mut uv_updates: Vec<(u32, usize, Vec2)> = Vec::new();
        if !shared_uvs.is_empty() {
            let approx = |u: Vec2, v: Vec2| (u.x - v.x).abs() < 1e-4 && (u.y - v.y).abs() < 1e-4;
            for &t in &self.vt[a as usize] {
                if shared.contains(&t) || !self.has_uv[t as usize] {
                    continue;
                }
                let ci = self.tv[t as usize].iter().position(|&v| v == a).unwrap();
                let cur_uv = self.tuv[t as usize][ci];
                if let Some(&(_, _, new_uv)) = shared_uvs.iter().find(|&&(ua, _, _)| approx(ua, cur_uv)) {
                    uv_updates.push((t, ci, new_uv));
                } else if t_param > 1e-5 {
                    return None;
                }
            }
            for &t in &self.vt[b as usize] {
                if shared.contains(&t) || !self.has_uv[t as usize] {
                    continue;
                }
                let ci = self.tv[t as usize].iter().position(|&v| v == b).unwrap();
                let cur_uv = self.tuv[t as usize][ci];
                if let Some(&(_, _, new_uv)) = shared_uvs.iter().find(|&&(_, ub, _)| approx(ub, cur_uv)) {
                    uv_updates.push((t, ci, new_uv));
                } else if t_param < 1.0 - 1e-5 {
                    return None;
                }
            }
        }

        for (t, ci, new_uv) in uv_updates {
            self.tuv[t as usize][ci] = new_uv;
        }
        for &t in &shared {
            self.alive[t as usize] = false;
            for v in self.tv[t as usize] {
                self.vt[v as usize].retain(|&x| x != t);
            }
        }
        let moved = std::mem::take(&mut self.vt[b as usize]);
        for t in moved {
            for v in &mut self.tv[t as usize] {
                if *v == b {
                    *v = a;
                }
            }
            self.vt[a as usize].push(t);
        }

        // Update feature-edge sets for edges moving from `b` to `a`.
        let ek = edge_key(a, b);
        self.boundary_edges.remove(&ek);
        self.seam_edges.remove(&ek);
        for &u in &nb {
            if u == a {
                continue;
            }
            let old_k = edge_key(b, u);
            let new_k = edge_key(a, u);
            if self.boundary_edges.remove(&old_k) {
                self.boundary_edges.insert(new_k);
            }
            if self.seam_edges.remove(&old_k) {
                self.seam_edges.insert(new_k);
            }
        }

        self.pos[a as usize] = p;
        let qb = self.q[b as usize];
        add_q(&mut self.q[a as usize], &qb);
        self.boundary[a as usize] |= self.boundary[b as usize];
        self.seam_vert[a as usize] |= self.seam_vert[b as usize];
        self.vdead[b as usize] = true;
        self.merged[b as usize] = a;
        self.version[a as usize] += 1;
        Some(shared.len())
    }
}

/// Quadric Error Metric mesh decimator preserving boundary contours and UV seams.
///
/// `target_ratio` in `0.0..=1.0` specifies the desired fraction of triangles to keep.
pub fn decimate_qem(mesh: &Mesh, target_ratio: f32) -> Mesh {
    let ratio = if target_ratio.is_finite() {
        target_ratio.clamp(0.0, 1.0)
    } else {
        1.0
    };
    if ratio >= 1.0 || mesh.faces.is_empty() {
        return mesh.clone();
    }

    let tris = mesh.triangulate();
    let nv = mesh.positions.len();
    let target = (tris.len() as f64 * ratio as f64).round().max(1.0) as usize;

    let mut tv = Vec::with_capacity(tris.len());
    let mut tuv = Vec::with_capacity(tris.len());
    let mut has_uv = Vec::with_capacity(tris.len());
    let mut tmat = Vec::with_capacity(tris.len());
    let mut tsmooth = Vec::with_capacity(tris.len());

    for t in &tris {
        let f = &mesh.faces[t.face as usize];
        let uv_ok = f.uvs.len() == f.verts.len();
        tv.push(t.verts);
        tuv.push(if uv_ok {
            t.corners.map(|c| f.uvs[c as usize])
        } else {
            [Vec2::ZERO; 3]
        });
        has_uv.push(uv_ok);
        tmat.push(f.material);
        tsmooth.push(f.smooth);
    }

    let mut st = QemState {
        pos: mesh.positions.iter().map(|p| [p.x as f64, p.y as f64, p.z as f64]).collect(),
        tv,
        tuv,
        has_uv,
        tmat,
        tsmooth,
        alive: vec![true; tris.len()],
        vt: vec![Vec::new(); nv],
        q: vec![[0.0; 10]; nv],
        boundary: vec![false; nv],
        seam_vert: vec![false; nv],
        seam_edges: HashSet::new(),
        boundary_edges: HashSet::new(),
        locked: vec![false; nv],
        version: vec![0; nv],
        vdead: vec![false; nv],
        merged: (0..nv as u32).collect(),
    };

    for (ti, t) in st.tv.iter().enumerate() {
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
            st.alive[ti] = false;
            continue;
        }
        for &v in t {
            st.vt[v as usize].push(ti as u32);
        }
    }

    let mut edge_tris: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for (ti, t) in st.tv.iter().enumerate() {
        if !st.alive[ti] {
            continue;
        }
        let pts = t.map(|v| st.pos[v as usize]);
        let n = p_cross(p_sub(pts[1], pts[0]), p_sub(pts[2], pts[0]));
        let l = p_len(n);
        if l > 0.0 {
            let n = p_scale(n, 1.0 / l);
            let q = plane_q(n, -p_dot(n, pts[0]), l * 0.5);
            for &v in t {
                add_q(&mut st.q[v as usize], &q);
            }
        }
        for i in 0..3 {
            edge_tris.entry(edge_key(t[i], t[(i + 1) % 3])).or_default().push(ti as u32);
        }
    }

    let mut feature_valence = vec![0u32; nv];
    let mut edges: Vec<(u32, u32)> = edge_tris.keys().copied().collect();
    edges.sort_unstable();

    let uv_match_on_edge = |t0: usize, t1: usize, a: u32, b: u32| -> bool {
        if !st.has_uv[t0] && !st.has_uv[t1] {
            return true;
        }
        if st.has_uv[t0] != st.has_uv[t1] {
            return false;
        }
        let get_uv = |ti: usize, v: u32| -> Option<Vec2> {
            st.tv[ti].iter().position(|&x| x == v).map(|i| st.tuv[ti][i])
        };
        match (get_uv(t0, a), get_uv(t1, a), get_uv(t0, b), get_uv(t1, b)) {
            (Some(ua0), Some(ua1), Some(ub0), Some(ub1)) => {
                (ua0.x - ua1.x).abs() < 1e-4
                    && (ua0.y - ua1.y).abs() < 1e-4
                    && (ub0.x - ub1.x).abs() < 1e-4
                    && (ub0.y - ub1.y).abs() < 1e-4
            }
            _ => false,
        }
    };

    for &(a, b) in &edges {
        let ts = &edge_tris[&(a, b)];
        let marked_seam = mesh.is_seam(a, b);
        match ts.len() {
            1 => {
                let t = st.tv[ts[0] as usize];
                let pts = t.map(|v| st.pos[v as usize]);
                let fnrm = p_cross(p_sub(pts[1], pts[0]), p_sub(pts[2], pts[0]));
                let e = p_sub(st.pos[b as usize], st.pos[a as usize]);
                let n = p_cross(e, fnrm);
                let l = p_len(n);
                if l > 0.0 {
                    let n = p_scale(n, 1.0 / l);
                    let q = plane_q(n, -p_dot(n, st.pos[a as usize]), QEM_BOUNDARY_WEIGHT * p_dot(e, e));
                    add_q(&mut st.q[a as usize], &q);
                    add_q(&mut st.q[b as usize], &q);
                }
                st.boundary[a as usize] = true;
                st.boundary[b as usize] = true;
                st.boundary_edges.insert((a, b));
                feature_valence[a as usize] += 1;
                feature_valence[b as usize] += 1;
            }
            2 => {
                let is_uv_seam = marked_seam || !uv_match_on_edge(ts[0] as usize, ts[1] as usize, a, b);
                if is_uv_seam {
                    let t = st.tv[ts[0] as usize];
                    let pts = t.map(|v| st.pos[v as usize]);
                    let fnrm = p_cross(p_sub(pts[1], pts[0]), p_sub(pts[2], pts[0]));
                    let e = p_sub(st.pos[b as usize], st.pos[a as usize]);
                    let n = p_cross(e, fnrm);
                    let l = p_len(n);
                    if l > 0.0 {
                        let n = p_scale(n, 1.0 / l);
                        let q = plane_q(n, -p_dot(n, st.pos[a as usize]), QEM_BOUNDARY_WEIGHT * p_dot(e, e));
                        add_q(&mut st.q[a as usize], &q);
                        add_q(&mut st.q[b as usize], &q);
                    }
                    st.seam_vert[a as usize] = true;
                    st.seam_vert[b as usize] = true;
                    st.seam_edges.insert((a, b));
                    feature_valence[a as usize] += 1;
                    feature_valence[b as usize] += 1;
                }
            }
            _ => {
                st.locked[a as usize] = true;
                st.locked[b as usize] = true;
            }
        }
    }
    // Lock feature corners/junctions where more than 2 seam/boundary edges meet.
    for v in 0..nv {
        if feature_valence[v] > 2 || (st.boundary[v] && st.seam_vert[v]) {
            st.locked[v] = true;
        }
    }

    let mut heap = BinaryHeap::with_capacity(edges.len());
    for &(a, b) in &edges {
        st.push(&mut heap, a, b);
    }

    let mut live = st.alive.iter().filter(|&&a| a).count();
    while live > target {
        let Some(e) = heap.pop() else { break };
        let (a, b) = (e.a, e.b);
        if st.vdead[a as usize]
            || st.vdead[b as usize]
            || e.stamp != (st.version[a as usize], st.version[b as usize])
        {
            continue;
        }
        let (p, _) = st.plan(a, b);
        let Some(removed) = st.collapse(a, b, p) else { continue };
        live -= removed;
        for u in st.neighbours(a) {
            st.push(&mut heap, a.min(u), a.max(u));
        }
    }

    let mut out = Mesh::new();
    out.positions = st
        .pos
        .iter()
        .map(|p| Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32))
        .collect();
    out.faces.reserve(live);
    for ti in 0..st.tv.len() {
        if !st.alive[ti] {
            continue;
        }
        out.faces.push(Face {
            verts: st.tv[ti].to_vec(),
            uvs: if st.has_uv[ti] { st.tuv[ti].to_vec() } else { Vec::new() },
            material: st.tmat[ti],
            smooth: st.tsmooth[ti],
        });
    }
    let find = |mut v: u32| {
        while st.merged[v as usize] != v {
            v = st.merged[v as usize];
        }
        v
    };
    out.seams = st
        .seam_edges
        .iter()
        .map(|&(a, b)| edge_key(find(a), find(b)))
        .filter(|e| e.0 != e.1)
        .collect();
    out.loose_edges = mesh
        .loose_edges
        .iter()
        .map(|e| [find(e[0]), find(e[1])])
        .filter(|e| e[0] != e[1])
        .collect();
    // Preserve point color attributes along merged vertices.
    for attr in &mesh.color_attributes {
        if attr.domain == AttrDomain::Point && attr.data.len() == nv {
            let idx = out.add_color_attribute(&attr.name, AttrDomain::Point, [1.0; 4]);
            out.color_attributes[idx].data = attr.data.clone();
            out.color_attributes[idx].byte = attr.byte;
        }
    }
    out.remove_unused_vertices();
    out.prune_flags();
    out.sanitize_attributes();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives;

    #[test]
    fn bounds_scale_and_volume_metrics() {
        let mut c = primitives::cube(2.0);
        scale_to_height_mm(&mut c, 50.0);
        let b = mesh_bounds_mm(&c);
        assert!((b.width - 50.0).abs() < 1e-3);
        assert!((b.depth - 50.0).abs() < 1e-3);
        assert!((b.height - 50.0).abs() < 1e-3);
        // 50 mm cube = 5 cm cube => 125 cm^3 volume, 150 cm^2 surface area.
        assert!((volume_cm3(&c) - 125.0).abs() < 1e-2);
        assert!((surface_area_cm2(&c) - 150.0).abs() < 1e-2);
        let metrics = print_metrics(&c);
        assert_eq!(metrics.triangle_count, 12);
        assert!(metrics.est_resin_g > 130.0 && metrics.est_filament_g > 150.0);
    }

    #[test]
    fn orient_upright_stands_tilted_and_inverted_shapes() {
        // Tilted cylinder (lying on X axis).
        let mut cyl = primitives::cylinder(16, 0.5, 4.0);
        cyl.transform(&Mat4::from_trs(Vec3::ZERO, Quat::from_axis_angle(Vec3::Y, core::f32::consts::FRAC_PI_2), Vec3::ONE));
        assert!(cyl.bounds().size().x > cyl.bounds().size().z);
        orient_upright(&mut cyl);
        let s = cyl.bounds().size();
        assert!(s.z > s.x * 2.0 && s.z > s.y * 2.0, "cylinder should be upright: {s:?}");

        // Inverted cone (base at +Z, tip at -Z).
        let mut cone = primitives::cone(16, 1.0, 0.0, 3.0);
        cone.transform(&Mat4::from_trs(Vec3::ZERO, Quat::from_axis_angle(Vec3::X, core::f32::consts::PI), Vec3::ONE));
        orient_upright(&mut cone);
        // Tip should point toward +Z, so max.z vertex count is 1 (apex) and min.z has the ring.
        let b = cone.bounds();
        let top_verts = cone.positions.iter().filter(|p| (p.z - b.max.z).abs() < 1e-3).count();
        let bot_verts = cone.positions.iter().filter(|p| (p.z - b.min.z).abs() < 1e-3).count();
        assert_eq!(top_verts, 1, "apex at +Z");
        assert!(bot_verts >= 16, "flat circular base at -Z");
    }

    #[test]
    fn place_on_bed_drops_to_zero_and_maximizes_contact() {
        // Tilted cube sitting on a corner above Z=5.
        let mut c = primitives::cube(10.0);
        c.transform(&Mat4::from_trs(Vec3::new(0.0, 0.0, 8.0), Quat::from_euler_xyz(0.5, 0.4, 0.1), Vec3::ONE));
        place_on_bed(&mut c);
        let b = c.bounds();
        assert!(b.min.z.abs() < 1e-4, "min.z = {}", b.min.z);
        assert!((b.size().z - 10.0).abs() < 1e-2, "cube flat on bed: {:?}", b.size());
        let on_bed = c.positions.iter().filter(|p| p.z.abs() < 1e-3).count();
        assert_eq!(on_bed, 4, "4 corners resting flat on bed");
    }

    #[test]
    fn inspect_and_repair_for_printing_closes_cube_and_fixes_normals() {
        let mut m = primitives::cube(10.0);
        // Remove one face, invert another, add a degenerate triangle, and split vertices.
        m.faces.pop();
        m.faces[0].verts.reverse();
        let a = m.add_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = m.add_vertex(Vec3::new(1.0, 0.0, 0.0));
        let c = m.add_vertex(Vec3::new(0.5, 0.0, 0.0));
        m.add_face(&[a, b, c]);
        // Duplicate vertex 0 very slightly offset on face 2 ([0, 1, 5, 4]).
        let dup = m.add_vertex(m.positions[0] + Vec3::new(1e-5, 0.0, 0.0));
        m.faces[2].verts[0] = dup;

        let bad = inspect_manifold(&m);
        assert!(!bad.is_watertight);
        assert!(bad.boundary_edges > 0);
        assert!(bad.degenerate_faces > 0);

        repair_for_printing(&mut m);
        let good = inspect_manifold(&m);
        assert!(good.is_watertight, "repaired report: {good:?}");
        assert_eq!(good.boundary_edges, 0);
        assert_eq!(good.non_manifold_edges, 0);
        assert_eq!(good.degenerate_faces, 0);
        assert!(good.outward_winding);
        assert!((volume_cm3(&m) - 1.0).abs() < 1e-2);
    }

    #[test]
    fn decimate_qem_preserves_boundary_and_uv_seams() {
        let g = crate::modifiers::Modifier::Triangulate.apply(&primitives::grid(12, 12, 10.0));
        let d = decimate_qem(&g, 0.35);
        d.validate().unwrap();
        assert!(d.face_count() < g.face_count() / 2);
        let b = d.bounds();
        assert!((b.size().x - 10.0).abs() < 1e-3 && (b.size().y - 10.0).abs() < 1e-3);
        assert!(d.has_uvs());

        // UV sphere preserves closed manifold and UVs.
        let s = primitives::uv_sphere(24, 12, 5.0);
        let ds = decimate_qem(&s, 0.5);
        ds.validate().unwrap();
        assert!(ds.has_uvs());
        let rep = inspect_manifold(&ds);
        assert!(rep.is_watertight, "decimated sphere must stay watertight: {rep:?}");
    }
}
