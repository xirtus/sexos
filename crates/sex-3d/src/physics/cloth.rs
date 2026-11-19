// SPDX-License-Identifier: GPL-3.0-only
//! Cloth and Soft Body: both are point masses on the mesh's vertices,
//! stepped with extended position-based dynamics (XPBD, small steps) in
//! world space, then handed back to the modifier stack in object space.
//!
//! Deviations from Blender: Blender's cloth uses an implicit mass-spring
//! solver (and soft bodies an explicit/adaptive RK one); here both are
//! XPBD constraint solves. Stiffness values map to compliances so the
//! defaults behave alike (Tension 15 is nearly inextensible, Bending 0.5
//! drapes), but numbers are not interchangeable with Blender's. The Angular
//! bending model is approximated by distance springs across faces like the
//! Linear one. Collisions push vertices out of collision objects' surfaces
//! (no edge/face continuous collision); self collision repels vertices
//! within the self distance using a spatial hash.
use super::collision::{colliders, Collider};
use super::field::Effectors;
use super::settings::{ClothSettings, SoftBodySettings};
use crate::math::{Mat4, Vec3};
use crate::mesh::Mesh;
use crate::modifiers::Modifier;
use crate::scene::Scene;
use std::collections::{HashMap, HashSet};

/// World-space vertex positions and velocities.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointsFrame {
    pub pos: Vec<Vec3>,
    pub vel: Vec<Vec3>,
    /// Where the modifier stack put the vertices (pin and goal targets).
    pub target: Vec<Vec3>,
}

impl PointsFrame {
    pub fn bytes(&self) -> usize {
        self.pos.len() * 36 + 48
    }
}

/// The mesh entering modifier `slot` of object `obj`: the stack above it.
pub fn stack_input(scene: &Scene, obj: usize, slot: usize) -> Option<Mesh> {
    let crate::scene::ObjectData::Mesh(mi) = scene.objects[obj].data else { return None };
    let stack = &scene.objects[obj].modifiers;
    let slot = slot.min(stack.len());
    Some(scene.apply_modifiers(obj, &scene.meshes[mi], &stack[..slot]))
}

fn to_world(m: &Mesh, mat: &Mat4) -> Vec<Vec3> {
    m.positions.iter().map(|&p| mat.transform_point(p)).collect()
}

struct Dist {
    a: usize,
    b: usize,
    rest: f32,
    /// Compliance when stretched / compressed.
    stretch: f32,
    compress: f32,
    /// Max force (sewing).
    damp: f32,
}

struct Pbd {
    x: Vec<Vec3>,
    v: Vec<Vec3>,
    w: Vec<f32>,
    dist: Vec<Dist>,
    /// (vertex, compliance) goal constraints towards `target`.
    goals: Vec<(usize, f32)>,
    target0: Vec<Vec3>,
    target1: Vec<Vec3>,
    /// Pinned vertices follow the target exactly.
    pinned: Vec<bool>,
    accel: Vec<Vec3>,
    air: f32,
    /// Collision distance and friction against objects.
    col_dist: f32,
    col_friction: f32,
    self_dist: Option<(f32, f32)>,
    pressure: f32,
    faces: Vec<Vec<u32>>,
    neighbours: HashSet<(u32, u32)>,
}

impl Pbd {
    fn run(&mut self, dt_frame: f32, substeps: usize, cols: &[Collider]) {
        if dt_frame <= 0.0 {
            return;
        }
        let n = self.x.len();
        let h = dt_frame / substeps as f32;
        let mut scratch = Vec::new();
        let mut prev = self.x.clone();
        for s in 0..substeps {
            let alpha = (s + 1) as f32 / substeps as f32;
            // Pressure as a uniform force along face normals.
            let mut press = vec![Vec3::ZERO; if self.pressure != 0.0 { n } else { 0 }];
            if self.pressure != 0.0 {
                for f in &self.faces {
                    if f.len() < 3 {
                        continue;
                    }
                    let c = f.iter().fold(Vec3::ZERO, |a, &v| a + self.x[v as usize]) / f.len() as f32;
                    let mut area = Vec3::ZERO;
                    for k in 0..f.len() {
                        let a = self.x[f[k] as usize] - c;
                        let b = self.x[f[(k + 1) % f.len()] as usize] - c;
                        area += a.cross(b) * 0.5;
                    }
                    let fv = area * (self.pressure / f.len() as f32);
                    for &v in f {
                        press[v as usize] += fv;
                    }
                }
            }
            let air = (1.0 - self.air * h).clamp(0.0, 1.0);
            for i in 0..n {
                prev[i] = self.x[i];
                if self.pinned[i] {
                    let t = self.target0[i].lerp(self.target1[i], alpha);
                    self.v[i] = (t - self.x[i]) / h;
                    self.x[i] = t;
                    continue;
                }
                let mut a = self.accel[i];
                if !press.is_empty() {
                    a += press[i] * self.w[i];
                }
                self.v[i] = (self.v[i] + a * h) * air;
                self.x[i] += self.v[i] * h;
            }
            let h2 = h * h;
            // Distance constraints.
            for c in &self.dist {
                let (wa, wb) = (self.w[c.a], self.w[c.b]);
                if wa + wb <= 0.0 {
                    continue;
                }
                let d = self.x[c.b] - self.x[c.a];
                let l = d.length();
                if l < 1e-9 {
                    continue;
                }
                let cval = l - c.rest;
                let comp = if cval > 0.0 { c.stretch } else { c.compress };
                if !comp.is_finite() {
                    continue;
                }
                let dl = -cval / (wa + wb + comp / h2);
                let mut corr = d / l * dl;
                if c.damp > 0.0 {
                    let m = corr.length();
                    if m > c.damp * h2 {
                        corr = corr * (c.damp * h2 / m);
                    }
                }
                self.x[c.a] -= corr * wa;
                self.x[c.b] += corr * wb;
            }
            // Goals.
            for &(i, comp) in &self.goals {
                if self.w[i] <= 0.0 {
                    continue;
                }
                let t = self.target0[i].lerp(self.target1[i], alpha);
                let d = self.x[i] - t;
                let k = self.w[i] / (self.w[i] + comp / h2);
                self.x[i] -= d * k;
            }
            // Self collision: repel close vertices that share no edge.
            if let Some((dist, fr)) = self.self_dist {
                let cell = dist.max(1e-4) * 2.0;
                let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
                let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
                for (i, &p) in self.x.iter().enumerate() {
                    grid.entry(key(p)).or_default().push(i as u32);
                }
                for i in 0..n {
                    let p = self.x[i];
                    let (kx, ky, kz) = key(p);
                    for dx in -1..=1 {
                        for dy in -1..=1 {
                            for dz in -1..=1 {
                                let Some(list) = grid.get(&(kx + dx, ky + dy, kz + dz)) else { continue };
                                for &j in list {
                                    let j = j as usize;
                                    if j <= i || self.neighbours.contains(&(i as u32, j as u32)) {
                                        continue;
                                    }
                                    let d = self.x[j] - self.x[i];
                                    let l = d.length();
                                    let ws = self.w[i] + self.w[j];
                                    if l < dist && l > 1e-9 && ws > 0.0 {
                                        let corr = d / l * ((dist - l) / ws);
                                        self.x[i] -= corr * self.w[i];
                                        self.x[j] += corr * self.w[j];
                                        let _ = fr;
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Object collisions.
            for i in 0..n {
                if self.pinned[i] || cols.is_empty() {
                    continue;
                }
                for c in cols {
                    let reach = self.col_dist + c.settings.thickness_outer + c.settings.thickness_inner;
                    let Some(hit) = c.closest(self.x[i], reach, &mut scratch) else { continue };
                    // Which side: where the vertex came from.
                    let mut nrm = hit.normal;
                    let from = prev[i] - hit.point;
                    if from.dot(nrm) < 0.0 && !c.settings.use_culling {
                        nrm = -nrm;
                    }
                    let want = self.col_dist + c.settings.thickness_outer;
                    let side = (self.x[i] - hit.point).dot(nrm);
                    if side < want {
                        let push = want - side;
                        self.x[i] += nrm * push;
                        // Friction: damp the tangential motion of this substep.
                        let mu = (self.col_friction.max(c.settings.cloth_friction) * 0.02).clamp(0.0, 1.0);
                        let mv = self.x[i] - prev[i];
                        let tang = mv - nrm * mv.dot(nrm);
                        self.x[i] -= tang * mu;
                    }
                }
            }
            // Velocities.
            for i in 0..n {
                if !self.pinned[i] {
                    self.v[i] = (self.x[i] - prev[i]) / h;
                }
            }
        }
    }
}

fn group_weights(m: &Mesh, name: &str) -> Option<Vec<f32>> {
    if name.is_empty() {
        return None;
    }
    m.vertex_group_index(name)?;
    let mut w = m.group_weights(name);
    w.resize(m.positions.len(), 0.0);
    Some(w)
}

/// Spring topology: edges, shear diagonals and bending pairs.
fn springs(m: &Mesh) -> (Vec<[u32; 2]>, Vec<[u32; 2]>, Vec<[u32; 2]>) {
    let edges = m.edges();
    let mut shear = Vec::new();
    for f in &m.faces {
        if f.verts.len() == 4 {
            shear.push([f.verts[0], f.verts[2]]);
            shear.push([f.verts[1], f.verts[3]]);
        }
    }
    let mut efaces: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (fi, f) in m.faces.iter().enumerate() {
        for (a, b) in f.edges() {
            efaces.entry((a.min(b), a.max(b))).or_default().push(fi);
        }
    }
    let mut keys: Vec<(u32, u32)> = efaces.keys().copied().collect();
    keys.sort_unstable();
    let mut bend = Vec::new();
    for k in keys {
        let fs = &efaces[&k];
        if fs.len() != 2 {
            continue;
        }
        let mid = (m.positions[k.0 as usize] + m.positions[k.1 as usize]) * 0.5;
        let far = |fi: usize| m.faces[fi].verts.iter().copied().filter(|&v| v != k.0 && v != k.1).max_by(|&a, &b| m.positions[a as usize].distance(mid).partial_cmp(&m.positions[b as usize].distance(mid)).unwrap());
        if let (Some(a), Some(b)) = (far(fs[0]), far(fs[1])) {
            if a != b {
                bend.push([a, b]);
            }
        }
    }
    (edges, shear, bend)
}

/// Compliance for a Blender-style stiffness (higher is stiffer).
fn compliance(stiffness: f32, scale: f32) -> f32 {
    if stiffness <= 0.0 {
        f32::INFINITY
    } else {
        1.0 / (stiffness * scale)
    }
}

fn accelerations(scene: &Scene, obj: usize, x: &[Vec3], v: &[Vec3], w: &[f32], weights: &super::settings::EffectorWeights) -> Vec<Vec3> {
    let eff = Effectors::collect(scene, Some(obj), weights);
    let t = scene.frame_current as f32;
    x.iter()
        .zip(v)
        .zip(w)
        .map(|((&p, &vel), &wi)| {
            let mut a = eff.gravity;
            if !eff.is_empty() {
                a += eff.force(p, vel, t) * wi;
            }
            a
        })
        .collect()
}

pub fn cloth_key(obj: &str, slot: usize) -> String {
    format!("{obj}/cloth{slot}")
}

pub fn softbody_key(obj: &str, slot: usize) -> String {
    format!("{obj}/softbody{slot}")
}

/// The first state: the input mesh where it is.
pub fn initial(scene: &Scene, obj: usize, slot: usize) -> Option<PointsFrame> {
    let m = stack_input(scene, obj, slot)?;
    let pos = to_world(&m, &scene.world_matrix(obj));
    let vel = vec![Vec3::ZERO; pos.len()];
    Some(PointsFrame { target: pos.clone(), pos, vel })
}

/// Advance a cloth one frame.
pub fn cloth_step(scene: &Scene, obj: usize, slot: usize, s: &ClothSettings, prev: &PointsFrame) -> Option<PointsFrame> {
    let prev_target = &prev.target;
    let m = stack_input(scene, obj, slot)?;
    let target = to_world(&m, &scene.world_matrix(obj));
    if target.len() != prev.pos.len() {
        return Some(PointsFrame { vel: vec![Vec3::ZERO; target.len()], pos: target.clone(), target });
    }
    let n = target.len();
    let pin = group_weights(&m, &s.vertex_group_mass);
    let shrink_w = group_weights(&m, &s.vertex_group_shrink);
    let mass = s.mass.max(1e-4);
    let mut pinned = vec![false; n];
    let mut goals = Vec::new();
    let mut w = vec![1.0 / mass; n];
    if let Some(pw) = &pin {
        for i in 0..n {
            if pw[i] >= 0.999 {
                pinned[i] = true;
                w[i] = 0.0;
            } else if pw[i] > 0.0 {
                goals.push((i, compliance(s.pin_stiffness * pw[i], 200.0)));
            }
        }
    }
    let (edges, shear, bend) = springs(&m);
    // Rest lengths come from the input mesh (Blender's rest shape).
    let rest = |a: u32, b: u32| {
        let l = m.positions[a as usize].distance(m.positions[b as usize]);
        let sw = shrink_w.as_ref().map_or(1.0, |g| (g[a as usize] + g[b as usize]) * 0.5);
        let scale = scene.world_matrix(obj).mat3().scale().max_elem().max(1e-6);
        l * scale * (1.0 - s.shrink_min * sw)
    };
    let mut dist = Vec::new();
    let damp = 0.0;
    for e in &edges {
        dist.push(Dist { a: e[0] as usize, b: e[1] as usize, rest: rest(e[0], e[1]), stretch: compliance(s.tension_stiffness, 2000.0), compress: compliance(s.compression_stiffness, 2000.0), damp });
    }
    for e in &shear {
        let c = compliance(s.shear_stiffness, 400.0);
        dist.push(Dist { a: e[0] as usize, b: e[1] as usize, rest: rest(e[0], e[1]), stretch: c, compress: c, damp });
    }
    for e in &bend {
        let c = compliance(s.bending_stiffness, 4.0);
        dist.push(Dist { a: e[0] as usize, b: e[1] as usize, rest: rest(e[0], e[1]), stretch: c, compress: c, damp });
    }
    if s.use_sewing_springs {
        for e in &m.loose_edges {
            let max = if s.sewing_force_max > 0.0 { s.sewing_force_max } else { 0.0 };
            dist.push(Dist { a: e[0] as usize, b: e[1] as usize, rest: 0.0, stretch: compliance(s.tension_stiffness, 200.0), compress: f32::INFINITY, damp: max });
        }
    }
    let neighbours: HashSet<(u32, u32)> = edges.iter().chain(&shear).chain(&bend).map(|e| (e[0].min(e[1]), e[0].max(e[1]))).collect();
    let accel = accelerations(scene, obj, &prev.pos, &prev.vel, &w, &s.effector_weights);
    let cols = if s.use_collision { colliders(scene, Some(obj), &s.collision_collection) } else { Vec::new() };
    let mut sys = Pbd {
        x: prev.pos.clone(),
        v: prev.vel.clone(),
        w,
        dist,
        goals,
        target0: if prev_target.len() == n { prev_target.to_vec() } else { target.clone() },
        target1: target.clone(),
        pinned,
        accel,
        air: s.air_damping * 0.1 + s.tension_damping * 0.004,
        col_dist: s.distance_min,
        col_friction: 0.0,
        self_dist: s.use_self_collision.then_some((s.self_distance_min, s.self_friction)),
        pressure: if s.use_pressure { s.uniform_pressure_force } else { 0.0 },
        faces: m.faces.iter().map(|f| f.verts.clone()).collect(),
        neighbours,
    };
    let dt = s.time_scale / scene.fps.max(1e-3);
    let substeps = (s.quality.max(1) as usize * 2 * s.collision_quality.clamp(1, 4) as usize / 2).max(2);
    sys.run(dt, substeps, &cols);
    Some(PointsFrame { pos: sys.x, vel: sys.v, target })
}

/// Advance a soft body one frame.
pub fn softbody_step(scene: &Scene, obj: usize, slot: usize, s: &SoftBodySettings, prev: &PointsFrame) -> Option<PointsFrame> {
    let prev_target = &prev.target;
    let m = stack_input(scene, obj, slot)?;
    let target = to_world(&m, &scene.world_matrix(obj));
    if target.len() != prev.pos.len() {
        return Some(PointsFrame { vel: vec![Vec3::ZERO; target.len()], pos: target.clone(), target });
    }
    let n = target.len();
    let mass_w = group_weights(&m, &s.vertex_group_mass);
    let goal_w = group_weights(&m, &s.vertex_group_goal);
    let mut w: Vec<f32> = (0..n).map(|i| 1.0 / (s.mass * mass_w.as_ref().map_or(1.0, |g| g[i].max(0.01))).max(1e-4)).collect();
    let mut pinned = vec![false; n];
    let mut goals = Vec::new();
    if s.use_goal {
        for i in 0..n {
            let g = goal_w.as_ref().map_or(s.goal_default, |g| g[i]);
            let g = s.goal_min + (s.goal_max - s.goal_min) * g.clamp(0.0, 1.0);
            if g >= 0.999 {
                pinned[i] = true;
                w[i] = 0.0;
            } else if g > 0.0 {
                // Goal stiffness 0..1 maps onto a compliance.
                let k = g * s.goal_spring / (1.0 - s.goal_spring).max(1e-3);
                goals.push((i, compliance(k, 50.0)));
            }
        }
    }
    let mut dist = Vec::new();
    let (edges, shear, bend) = springs(&m);
    let scale = scene.world_matrix(obj).mat3().scale().max_elem().max(1e-6);
    let springk = |x: f32| compliance(x / (1.0 - x).max(1e-3), 200.0);
    if s.use_edges {
        let mut all: Vec<[u32; 2]> = edges.clone();
        all.extend(shear.iter().copied());
        for e in all {
            let rest = m.positions[e[0] as usize].distance(m.positions[e[1] as usize]) * scale * if s.spring_length > 0.0 { s.spring_length / 100.0 } else { 1.0 };
            dist.push(Dist { a: e[0] as usize, b: e[1] as usize, rest, stretch: springk(s.pull), compress: springk(s.push), damp: 0.0 });
        }
        if s.bend > 0.0 {
            for e in &bend {
                let rest = m.positions[e[0] as usize].distance(m.positions[e[1] as usize]) * scale;
                let c = compliance(s.bend, 50.0);
                dist.push(Dist { a: e[0] as usize, b: e[1] as usize, rest, stretch: c, compress: c, damp: 0.0 });
            }
        }
    }
    let neighbours: HashSet<(u32, u32)> = edges.iter().map(|e| (e[0].min(e[1]), e[0].max(e[1]))).collect();
    let accel = accelerations(scene, obj, &prev.pos, &prev.vel, &w, &s.effector_weights);
    let cols = colliders(scene, Some(obj), &s.collision_collection);
    let mut sys = Pbd {
        x: prev.pos.clone(),
        v: prev.vel.clone(),
        w,
        dist,
        goals,
        target0: if prev_target.len() == n { prev_target.to_vec() } else { target.clone() },
        target1: target.clone(),
        pinned,
        accel,
        air: s.damping * 0.05 + s.goal_friction * 0.05,
        col_dist: 0.0,
        col_friction: s.friction,
        self_dist: s.use_self_collision.then_some((s.ball_size * 0.2, 0.0)),
        pressure: 0.0,
        faces: Vec::new(),
        neighbours,
    };
    let dt = s.speed / scene.fps.max(1e-3);
    let substeps = (s.step_min.max(1) as usize).clamp(2, 60);
    sys.run(dt, substeps, &cols);
    Some(PointsFrame { pos: sys.x, vel: sys.v, target })
}

/// The Cloth or Soft Body modifier at slot `slot`: the cached positions at the
/// scene's frame in object space, or the mesh unchanged.
pub fn modifier_eval(m: &Mesh, scene: &Scene, obj: usize, slot: usize, key: &str, start: i32) -> Mesh {
    if scene.frame_current < start {
        return m.clone();
    }
    let Some(f) = scene.physics.caches.frame_at(key, scene.frame_current) else { return m.clone() };
    let super::cache::SimFrame::Points(p) = &*f else { return m.clone() };
    if p.pos.len() != m.positions.len() {
        return m.clone();
    }
    let _ = slot;
    let inv = scene.world_matrix(obj).inverse();
    let mut out = m.clone();
    for (d, &s) in out.positions.iter_mut().zip(&p.pos) {
        *d = inv.transform_point(s);
    }
    out
}

/// The Cloth/Soft Body modifier slots of an object: (slot, key).
pub fn sim_slots(scene: &Scene, obj: usize) -> Vec<(usize, String, bool)> {
    let name = &scene.objects[obj].name;
    scene.objects[obj]
        .modifiers
        .iter()
        .enumerate()
        .filter_map(|(i, md)| match md {
            Modifier::Physics(super::PhysicsModifier::Cloth(_)) => Some((i, cloth_key(name, i), true)),
            Modifier::Physics(super::PhysicsModifier::SoftBody(_)) => Some((i, softbody_key(name, i), false)),
            _ => None,
        })
        .collect()
}
