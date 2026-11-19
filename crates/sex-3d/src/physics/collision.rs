// SPDX-License-Identifier: GPL-3.0-only
//! Collision objects (objects with a Collision modifier) as cloth, soft
//! bodies and particles see them: world-space triangles with an AABB tree
//! for closest-point and segment queries.
use super::rigid::shape::TriMesh;
use super::settings::CollisionSettings;
use super::PhysicsModifier;
use crate::math::{Aabb, Vec3};
use crate::modifiers::Modifier;
use crate::scene::Scene;

pub struct Collider {
    pub object: usize,
    pub settings: CollisionSettings,
    pub mesh: TriMesh,
    pub normals: Vec<Vec3>,
}

/// Closest point on triangle `abc` to `p` (Ericson).
pub fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let den = 1.0 / (va + vb + vc);
    a + ab * (vb * den) + ac * (vc * den)
}

/// A hit: point, unit normal of the triangle, distance or ray parameter.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub point: Vec3,
    pub normal: Vec3,
    pub t: f32,
}

impl Collider {
    /// The closest surface point within `radius` of `p`.
    pub fn closest(&self, p: Vec3, radius: f32, scratch: &mut Vec<usize>) -> Option<Hit> {
        scratch.clear();
        let q = Aabb { min: p - Vec3::splat(radius), max: p + Vec3::splat(radius) };
        self.mesh.query(&q, scratch);
        let mut best: Option<Hit> = None;
        for &t in scratch.iter() {
            let [a, b, c] = self.mesh.tri(t);
            let x = closest_on_triangle(p, a, b, c);
            let d = x.distance(p);
            if d <= radius && best.is_none_or(|h| d < h.t) {
                best = Some(Hit { point: x, normal: self.normals[t], t: d });
            }
        }
        best
    }

    /// First triangle crossed by the segment `a..b` (t in 0..1).
    pub fn segment(&self, a: Vec3, b: Vec3, pad: f32, scratch: &mut Vec<usize>) -> Option<Hit> {
        scratch.clear();
        let q = Aabb { min: a.min(b) - Vec3::splat(pad), max: a.max(b) + Vec3::splat(pad) };
        self.mesh.query(&q, scratch);
        let d = b - a;
        let mut best: Option<Hit> = None;
        for &t in scratch.iter() {
            let [p0, p1, p2] = self.mesh.tri(t);
            let e1 = p1 - p0;
            let e2 = p2 - p0;
            let h = d.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-12 {
                continue;
            }
            let inv = 1.0 / det;
            let s = a - p0;
            let u = s.dot(h) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = s.cross(e1);
            let v = d.dot(qv) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let tt = e2.dot(qv) * inv;
            if (0.0..=1.0).contains(&tt) && best.is_none_or(|h| tt < h.t) {
                best = Some(Hit { point: a + d * tt, normal: self.normals[t], t: tt });
            }
        }
        best
    }
}

/// The collision settings of object `i`, if it has an enabled Collision modifier.
pub fn collision_settings(scene: &Scene, i: usize) -> Option<&CollisionSettings> {
    scene.objects[i].modifiers.iter().find_map(|m| match m {
        Modifier::Physics(PhysicsModifier::Collision(c)) if c.use_collision => Some(c),
        _ => None,
    })
}

/// Every collision object but `exclude`, limited to `collection` when set.
pub fn colliders(scene: &Scene, exclude: Option<usize>, collection: &str) -> Vec<Collider> {
    let coll = if collection.is_empty() { None } else { scene.collections.iter().position(|c| c.name == collection) };
    let mut out = Vec::new();
    for i in 0..scene.objects.len() {
        if Some(i) == exclude || !scene.is_visible(i) {
            continue;
        }
        if let Some(c) = coll {
            if scene.objects[i].collection != c {
                continue;
            }
        }
        let Some(cs) = collision_settings(scene, i) else { continue };
        let Some((m, _)) = scene.world_mesh(i) else { continue };
        let tris: Vec<[u32; 3]> = m.triangulate().iter().map(|t| t.verts).collect();
        if tris.is_empty() {
            continue;
        }
        let normals = tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| m.positions[v as usize]);
                let n = (b - a).cross(c - a);
                if n.length_squared() > 0.0 {
                    n.normalize()
                } else {
                    Vec3::Z
                }
            })
            .collect();
        out.push(Collider { object: i, settings: cs.clone(), mesh: TriMesh::new(m.positions.clone(), tris), normals });
    }
    out
}
