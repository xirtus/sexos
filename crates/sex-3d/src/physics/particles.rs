// SPDX-License-Identifier: GPL-3.0-only
//! Particle systems: emitters (Newtonian particles with collisions and
//! fields, cached frame by frame) and static hair, with Simple and
//! Interpolated children, rendered as halos, lines, object or collection
//! instances, or strands.
//!
//! Everything random comes from the system's seed and the particle index,
//! so a system emits the same particles every time. Births are spread
//! evenly between Frame Start and End (Blender's default timing).
use super::cache::SimFrame;
use super::collision::colliders;
use super::field::Effectors;
use super::settings::*;
use super::PhysicsModifier;
use crate::math::{Mat4, Quat, Vec3};
use crate::mesh::Mesh;
use crate::modifiers::Modifier;
use crate::scene::{ObjectData, Scene};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alive {
    Unborn,
    Alive,
    Dead,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub ang: Vec3,
    pub size: f32,
    pub birth: f32,
    pub die: f32,
    pub state: Alive,
}

/// A cached frame: every particle (unborn ones at their emitter).
#[derive(Clone, Debug, Default)]
pub struct ParticlesFrame {
    pub particles: Vec<Particle>,
    /// The emitter's world matrix at this frame (object velocity).
    pub emitter: Option<Mat4>,
}

impl ParticlesFrame {
    pub fn bytes(&self) -> usize {
        self.particles.len() * std::mem::size_of::<Particle>() + 80
    }
    pub fn alive(&self) -> usize {
        self.particles.iter().filter(|p| p.state == Alive::Alive).count()
    }
}

/// Deterministic random numbers per (seed, particle, purpose).
pub fn rand(seed: i32, i: u32, salt: u32) -> f32 {
    let mut z = (seed as u32 as u64) << 32 ^ (i as u64).wrapping_mul(0x9E37_79B9) ^ (salt as u64) << 48;
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32
}

fn rand_unit(seed: i32, i: u32, salt: u32) -> Vec3 {
    let z = rand(seed, i, salt) * 2.0 - 1.0;
    let a = rand(seed, i, salt + 1) * std::f32::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(r * a.cos(), r * a.sin(), z)
}

pub fn key(obj: &str, slot: usize) -> String {
    format!("{obj}/particles{slot}")
}

/// Where a particle comes from on the emitter, in object space.
#[derive(Clone, Copy, Debug)]
pub struct Source {
    pub pos: Vec3,
    pub normal: Vec3,
    pub tangent: Vec3,
}

/// Emission sources for particles `0..count` (object space).
pub fn sources(m: &Mesh, s: &ParticleSettings, seed: i32, count: usize, salt: u32) -> Vec<Source> {
    let mut out = Vec::with_capacity(count);
    if m.positions.is_empty() || count == 0 {
        return out;
    }
    let vn = m.vertex_normals();
    match s.emit_from {
        EmitFrom::Vert => {
            let nv = m.positions.len();
            for i in 0..count {
                let v = if s.use_emit_random { (rand(seed, i as u32, salt) * nv as f32) as usize % nv } else { i % nv };
                let n = vn[v];
                out.push(Source { pos: m.positions[v], normal: n, tangent: n.any_orthonormal() });
            }
        }
        EmitFrom::Face | EmitFrom::Volume if m.faces.is_empty() => {
            return sources(m, &ParticleSettings { emit_from: EmitFrom::Vert, ..s.clone() }, seed, count, salt);
        }
        EmitFrom::Face => {
            let tris = m.triangulate();
            let area: Vec<f32> = tris
                .iter()
                .map(|t| {
                    let [a, b, c] = t.verts.map(|v| m.positions[v as usize]);
                    if s.use_even_distribution {
                        0.5 * (b - a).cross(c - a).length()
                    } else {
                        1.0
                    }
                })
                .collect();
            let total: f32 = area.iter().sum::<f32>().max(1e-12);
            let mut cdf = Vec::with_capacity(area.len());
            let mut acc = 0.0;
            for a in &area {
                acc += a / total;
                cdf.push(acc);
            }
            for i in 0..count {
                // Jittered: stratified along the area; Random: independent.
                let u = match s.distribution {
                    Distribution::Random => rand(seed, i as u32, salt),
                    _ => {
                        let k = if s.use_emit_random { (rand(seed, i as u32, salt + 9) * count as f32) as usize % count } else { i };
                        (k as f32 + 0.5 + (rand(seed, i as u32, salt) - 0.5) * s.jitter_factor.min(1.0)) / count as f32
                    }
                };
                let ti = cdf.partition_point(|&c| c < u).min(tris.len() - 1);
                let t = &tris[ti];
                let [a, b, c] = t.verts.map(|v| m.positions[v as usize]);
                let (mut r1, mut r2) = (rand(seed, i as u32, salt + 2), rand(seed, i as u32, salt + 3));
                if r1 + r2 > 1.0 {
                    r1 = 1.0 - r1;
                    r2 = 1.0 - r2;
                }
                let p = a + (b - a) * r1 + (c - a) * r2;
                let n = (b - a).cross(c - a);
                let n = if n.length_squared() > 0.0 { n.normalize() } else { Vec3::Z };
                let tan = (b - a).normalize();
                out.push(Source { pos: p, normal: n, tangent: tan });
            }
        }
        EmitFrom::Volume => {
            let b = m.bounds();
            let tris: Vec<[Vec3; 3]> = m.triangulate().iter().map(|t| t.verts.map(|v| m.positions[v as usize])).collect();
            let inside = |p: Vec3| {
                // Parity along +X.
                let mut n = 0;
                for [a, bb, c] in &tris {
                    if let Some(t) = ray_tri(p, Vec3::new(1.0, 0.0013, 0.0007), *a, *bb, *c) {
                        if t > 0.0 {
                            n += 1;
                        }
                    }
                }
                n % 2 == 1
            };
            for i in 0..count {
                let mut p = b.center();
                for k in 0..64u32 {
                    let q = Vec3::new(
                        b.min.x + (b.max.x - b.min.x) * rand(seed, i as u32, salt + 10 + 3 * k),
                        b.min.y + (b.max.y - b.min.y) * rand(seed, i as u32, salt + 11 + 3 * k),
                        b.min.z + (b.max.z - b.min.z) * rand(seed, i as u32, salt + 12 + 3 * k),
                    );
                    if inside(q) {
                        p = q;
                        break;
                    }
                }
                let n = rand_unit(seed, i as u32, salt + 5);
                out.push(Source { pos: p, normal: n, tangent: n.any_orthonormal() });
            }
        }
    }
    out
}

fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let h = d.cross(e2);
    let det = e1.dot(h);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(h) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(e2.dot(q) * inv)
}

/// The emitter mesh (modifier stack above the particle system, or the
/// base mesh), object space.
pub fn emitter_mesh(scene: &Scene, obj: usize, slot: usize) -> Option<Mesh> {
    super::cloth::stack_input(scene, obj, slot)
}

fn birth_time(s: &ParticleSettings, i: usize, count: usize) -> f32 {
    let span = s.frame_end - s.frame_start;
    if count <= 1 || span <= 0.0 {
        return s.frame_start;
    }
    s.frame_start + span * (i as f32 / count as f32)
}

/// Unborn particles with their birth and death times.
pub fn initial(scene: &Scene, obj: usize, slot: usize, sys: &ParticleSystem) -> ParticlesFrame {
    let s = &sys.settings;
    let n = s.count.max(0) as usize;
    let m = scene.world_matrix(obj);
    let mut particles = Vec::with_capacity(n);
    for i in 0..n {
        let birth = birth_time(s, i, n);
        let life = s.lifetime * (1.0 - s.lifetime_random * rand(sys.seed, i as u32, 101));
        let size = s.particle_size * (1.0 - s.size_random * rand(sys.seed, i as u32, 102));
        particles.push(Particle { pos: m.transform_point(Vec3::ZERO), vel: Vec3::ZERO, rot: Quat::IDENTITY, ang: Vec3::ZERO, size, birth, die: birth + life.max(1e-3), state: Alive::Unborn });
    }
    let _ = slot;
    ParticlesFrame { particles, emitter: Some(m) }
}

struct Env<'a> {
    s: &'a ParticleSettings,
    eff: Effectors,
    seed: i32,
}

impl Env<'_> {
    fn accel(&self, p: &Particle, x: Vec3, v: Vec3, t: f32, i: usize) -> Vec3 {
        let s = self.s;
        let mass = if s.use_multiply_size_mass { s.mass * p.size } else { s.mass }.max(1e-6);
        let mut f = self.eff.force(x, v, t);
        if s.drag_factor > 0.0 {
            f -= v * (s.drag_factor * p.size * p.size * v.length() * std::f32::consts::PI);
        }
        let mut a = self.eff.gravity + f / mass;
        if s.brownian_factor > 0.0 {
            a += rand_unit(self.seed, i as u32, 3000 + (t * 97.0) as u32) * (s.brownian_factor * 50.0);
        }
        a
    }

    fn integrate(&self, p: &mut Particle, dt: f32, t: f32, i: usize) {
        let (x, v) = (p.pos, p.vel);
        match self.s.integrator {
            Integrator::Euler | Integrator::Verlet => {
                let a = self.accel(p, x, v, t, i);
                p.vel = v + a * dt;
                p.pos = x + p.vel * dt;
            }
            Integrator::Midpoint => {
                let a0 = self.accel(p, x, v, t, i);
                let xm = x + v * (dt * 0.5);
                let vm = v + a0 * (dt * 0.5);
                let am = self.accel(p, xm, vm, t, i);
                p.pos = x + vm * dt;
                p.vel = v + am * dt;
            }
            Integrator::Rk4 => {
                let k1v = self.accel(p, x, v, t, i);
                let k1x = v;
                let k2v = self.accel(p, x + k1x * (dt * 0.5), v + k1v * (dt * 0.5), t, i);
                let k2x = v + k1v * (dt * 0.5);
                let k3v = self.accel(p, x + k2x * (dt * 0.5), v + k2v * (dt * 0.5), t, i);
                let k3x = v + k2v * (dt * 0.5);
                let k4v = self.accel(p, x + k3x * dt, v + k3v * dt, t, i);
                let k4x = v + k3v * dt;
                p.pos = x + (k1x + k2x * 2.0 + k3x * 2.0 + k4x) * (dt / 6.0);
                p.vel = v + (k1v + k2v * 2.0 + k3v * 2.0 + k4v) * (dt / 6.0);
            }
        }
        if self.s.damping > 0.0 {
            p.vel *= 1.0 - self.s.damping.min(1.0);
        }
        if p.ang != Vec3::ZERO {
            let w = p.ang;
            let l = w.length();
            p.rot = (Quat::from_axis_angle(w / l, l * dt) * p.rot).normalize();
        }
    }
}

/// The particle born from source `src` at its birth (emitter at `m`,
/// moving with `emitter_vel`).
fn birth(p: &mut Particle, sys: &ParticleSystem, i: usize, src: &Source, m: &Mat4, emitter_vel: Vec3) {
    let s = &sys.settings;
    let seed = sys.seed;
    let pos = m.transform_point(src.pos);
    let n = m.transform_vector(src.normal).normalize();
    let t0 = m.transform_vector(src.tangent).normalize();
    let tan = Quat::from_axis_angle(n, s.tangent_phase * std::f32::consts::PI).rotate(t0);
    let mut vel = n * s.normal_factor + tan * s.tangent_factor + emitter_vel * s.object_factor;
    vel += m.transform_vector(Vec3::from_array(s.object_align_factor));
    if s.factor_random > 0.0 {
        vel += rand_unit(seed, i as u32, 200) * (s.factor_random * rand(seed, i as u32, 203));
    }
    p.pos = pos;
    p.vel = vel;
    p.state = Alive::Alive;
    if s.use_rotations {
        let axis = match s.rotation_mode {
            RotationMode::None => Vec3::X,
            RotationMode::Normal | RotationMode::NormalTangent => n,
            RotationMode::Velocity => {
                if vel.length_squared() > 0.0 {
                    vel.normalize()
                } else {
                    n
                }
            }
            RotationMode::GlobalX => Vec3::X,
            RotationMode::GlobalY => Vec3::Y,
            RotationMode::GlobalZ => Vec3::Z,
        };
        let base = Quat::from_rotation_arc(Vec3::X, axis);
        let phase = Quat::from_axis_angle(axis, (s.phase_factor + s.phase_factor_random * (rand(seed, i as u32, 210) - 0.5)) * std::f32::consts::PI);
        let rnd = Quat::from_axis_angle(rand_unit(seed, i as u32, 211), s.rotation_factor_random * std::f32::consts::PI * rand(seed, i as u32, 214));
        p.rot = (rnd * phase * base).normalize();
        p.ang = match s.angular_velocity_mode {
            AngularVelocityMode::None => Vec3::ZERO,
            AngularVelocityMode::Velocity => {
                if vel.length_squared() > 0.0 {
                    vel.normalize() * s.angular_velocity_factor
                } else {
                    Vec3::ZERO
                }
            }
            AngularVelocityMode::Random => rand_unit(seed, i as u32, 220) * s.angular_velocity_factor,
            AngularVelocityMode::GlobalZ => Vec3::Z * s.angular_velocity_factor,
        };
    }
}

/// Advance an emitter one frame (`frame - 1` to `frame`).
pub fn step(scene: &Scene, obj: usize, slot: usize, sys: &ParticleSystem, prev: &ParticlesFrame, frame: i32) -> ParticlesFrame {
    let s = &sys.settings;
    let mut out = prev.clone();
    let n = s.count.max(0) as usize;
    if out.particles.len() != n {
        out = initial(scene, obj, slot, sys);
    }
    let m = scene.world_matrix(obj);
    let m0 = prev.emitter.unwrap_or(m);
    let mesh = emitter_mesh(scene, obj, slot).unwrap_or_default();
    let fps = scene.fps.max(1e-3);
    let dt_frame = s.timestep.max(1e-5) * 25.0 / fps;
    let emitter_vel = (m.transform_point(Vec3::ZERO) - m0.transform_point(Vec3::ZERO)) / dt_frame.max(1e-6);
    let (f0, f1) = ((frame - 1) as f32, frame as f32);
    let newborn: Vec<usize> = (0..n).filter(|&i| out.particles[i].state == Alive::Unborn && out.particles[i].birth <= f1).collect();
    let srcs = if newborn.is_empty() { Vec::new() } else { sources(&mesh, s, sys.seed, n, 0) };
    for &i in &newborn {
        let src = srcs.get(i).copied().unwrap_or(Source { pos: Vec3::ZERO, normal: Vec3::Z, tangent: Vec3::X });
        // The emitter where it was at the birth time.
        let f = ((out.particles[i].birth - f0).clamp(0.0, 1.0)).max(0.0);
        let mb = m0.interpolate(&m, f);
        birth(&mut out.particles[i], sys, i, &src, &mb, emitter_vel);
    }
    let physics = s.physics_type == ParticlePhysics::Newton;
    let env = Env { s, eff: Effectors::collect(scene, Some(obj), &s.effector_weights), seed: sys.seed };
    let cols = if physics { colliders(scene, Some(obj), &s.collision_collection) } else { Vec::new() };
    let sub = s.subframes.max(0) as usize + 1;
    let mut scratch = Vec::new();
    for (i, p) in out.particles.iter_mut().enumerate() {
        if p.state != Alive::Alive {
            continue;
        }
        if p.die <= f1 && p.die > f0 {
            // Dies this frame after moving until then.
        } else if p.die <= f0 {
            p.state = Alive::Dead;
            continue;
        }
        let start = p.birth.max(f0);
        let end = p.die.min(f1);
        let span = (end - start).max(0.0);
        if physics && span > 0.0 {
            let h = dt_frame * span / sub as f32;
            for k in 0..sub {
                let before = p.pos;
                env.integrate(p, h, start + span * k as f32 / sub as f32, i);
                for c in &cols {
                    let pad = if s.use_size_deflect { p.size } else { 0.0 };
                    let Some(hit) = c.segment(before, p.pos, pad + 1e-4, &mut scratch) else { continue };
                    let cs = &c.settings;
                    if cs.permeability > 0.0 && rand(sys.seed, i as u32, 400 + k as u32 + (f1 as u32) * 7) < cs.permeability {
                        continue;
                    }
                    if cs.use_particle_kill || s.use_die_on_collision {
                        p.pos = hit.point;
                        p.state = Alive::Dead;
                        p.die = start + span * (k as f32 + hit.t) / sub as f32;
                        break;
                    }
                    let mut nrm = hit.normal;
                    if (before - hit.point).dot(nrm) < 0.0 {
                        nrm = -nrm;
                    }
                    let damp = (cs.damping_factor + cs.damping_random * (rand(sys.seed, i as u32, 500) - 0.5)).clamp(0.0, 1.0);
                    let fric = (cs.friction_factor + cs.friction_random * (rand(sys.seed, i as u32, 501) - 0.5)).clamp(0.0, 1.0);
                    let vn = nrm * p.vel.dot(nrm);
                    let vt = p.vel - vn;
                    p.vel = vt * (1.0 - fric) - vn * (1.0 - damp);
                    p.pos = hit.point + nrm * (pad + 1e-3);
                }
                if p.state == Alive::Dead {
                    break;
                }
            }
        } else if !physics {
            // "None": particles stay where they were born.
        }
        if p.die <= f1 {
            p.state = Alive::Dead;
        }
    }
    out.emitter = Some(m);
    out
}

/// The particle systems of object `obj`: (modifier slot, key, system).
pub fn systems(scene: &Scene, obj: usize) -> Vec<(usize, String, &ParticleSystem)> {
    let name = &scene.objects[obj].name;
    scene.objects[obj]
        .modifiers
        .iter()
        .enumerate()
        .filter_map(|(i, md)| match md {
            Modifier::Physics(PhysicsModifier::ParticleSystem(p)) => Some((i, key(name, i), &**p)),
            _ => None,
        })
        .collect()
}

/// A particle as drawn or rendered.
#[derive(Clone, Copy, Debug)]
pub struct Point {
    pub pos: Vec3,
    pub vel: Vec3,
    pub rot: Quat,
    pub size: f32,
    pub index: usize,
}

/// What a particle system shows at the scene's frame.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub points: Vec<Point>,
    /// World-space strands (hair, with children).
    pub strands: Vec<Vec<Vec3>>,
}

/// Hair strands in object space: parents, then children.
pub fn hair_strands(mesh: &Mesh, sys: &ParticleSystem, render: bool) -> Vec<Vec<Vec3>> {
    let s = &sys.settings;
    let n = s.count.max(0) as usize;
    let srcs = sources(mesh, s, sys.seed, n, 0);
    let segs = s.hair_step.max(1) as usize;
    let mut parents = Vec::with_capacity(n);
    for (i, src) in srcs.iter().enumerate() {
        let mut dir = src.normal * s.normal_factor + src.tangent * s.tangent_factor + Vec3::from_array(s.object_align_factor);
        if s.factor_random > 0.0 {
            dir += rand_unit(sys.seed, i as u32, 200) * (s.factor_random * rand(sys.seed, i as u32, 203));
        }
        let dir = if dir.length_squared() > 1e-12 { dir.normalize() } else { src.normal };
        let pts: Vec<Vec3> = (0..=segs).map(|k| src.pos + dir * (s.hair_length * k as f32 / segs as f32)).collect();
        parents.push((pts, *src));
    }
    let mut out: Vec<Vec<Vec3>> = parents.iter().map(|p| p.0.clone()).collect();
    let per = if render { s.rendered_child_count } else { s.child_nbr }.max(0) as usize;
    match s.child_type {
        ChildType::None => {}
        ChildType::Simple => {
            for (pi, (pts, src)) in parents.iter().enumerate() {
                let t1 = src.tangent;
                let t2 = src.normal.cross(t1);
                for c in 0..per {
                    let id = (pi * per + c) as u32;
                    let a = rand(sys.seed, id, 700) * std::f32::consts::TAU;
                    let r = s.child_radius * rand(sys.seed, id, 701).sqrt();
                    let off = (t1 * a.cos() + t2 * a.sin()) * r;
                    out.push(child_strand(pts, off, s, sys.seed, id));
                }
            }
        }
        ChildType::Interpolated => {
            let total = n * per;
            let csrc = sources(mesh, s, sys.seed.wrapping_add(1), total, 50);
            for (c, src) in csrc.iter().enumerate() {
                // Three nearest parents, inverse-distance weighted.
                let mut near: Vec<(f32, usize)> = parents.iter().enumerate().map(|(k, p)| (p.1.pos.distance(src.pos), k)).collect();
                near.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                near.truncate(3);
                let wsum: f32 = near.iter().map(|(d, _)| 1.0 / (d + 1e-4)).sum();
                let mut pts = vec![Vec3::ZERO; segs + 1];
                for &(d, k) in &near {
                    let w = 1.0 / (d + 1e-4) / wsum;
                    let pp = &parents[k].0;
                    for (j, p) in pts.iter_mut().enumerate() {
                        *p += (pp[j] - pp[0]) * w;
                    }
                }
                let strand: Vec<Vec3> = pts.iter().map(|&d| src.pos + d).collect();
                out.push(child_strand(&strand, Vec3::ZERO, s, sys.seed, c as u32 + 100_000));
            }
        }
    }
    out
}

fn child_strand(parent: &[Vec3], off: Vec3, s: &ParticleSettings, seed: i32, id: u32) -> Vec<Vec3> {
    let n = parent.len();
    let len = s.child_length.clamp(0.0, 1.0);
    parent
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let t = if n > 1 { k as f32 / (n - 1) as f32 } else { 0.0 };
            let clump = if s.clump_factor >= 0.0 { 1.0 - s.clump_factor * t } else { 1.0 + s.clump_factor * (1.0 - t) };
            let mut q = parent[0] + (p - parent[0]) * len + off * clump;
            if s.roughness_1 > 0.0 {
                q += super::field::noise3(q * 8.0 + Vec3::splat(id as f32 * 0.37 + seed as f32)) * (s.roughness_1 * 0.1 * t);
            }
            q
        })
        .collect()
}

/// The particles of system `slot` on `obj` at the scene's frame.
pub fn view(scene: &Scene, obj: usize, slot: usize, sys: &ParticleSystem, render: bool) -> View {
    let s = &sys.settings;
    let m = scene.world_matrix(obj);
    if s.kind == ParticleType::Hair {
        let Some(mesh) = emitter_mesh(scene, obj, slot) else { return View::default() };
        let strands = hair_strands(&mesh, sys, render).into_iter().map(|st| st.into_iter().map(|p| m.transform_point(p)).collect()).collect();
        return View { points: Vec::new(), strands };
    }
    let k = key(&scene.objects[obj].name, slot);
    let Some(f) = scene.physics.caches.frame_at(&k, scene.frame_current) else { return View::default() };
    let SimFrame::Particles(pf) = &*f else { return View::default() };
    let pct = if render { 100 } else { s.display_percentage.clamp(0, 100) } as usize;
    let mut points = Vec::new();
    for (i, p) in pf.particles.iter().enumerate() {
        if pct < 100 && rand(sys.seed, i as u32, 900) * 100.0 >= pct as f32 {
            continue;
        }
        let show = match p.state {
            Alive::Alive => true,
            Alive::Unborn => s.show_unborn,
            Alive::Dead => s.use_dead,
        };
        if show {
            let pos = if p.state == Alive::Unborn { m.transform_point(Vec3::ZERO) } else { p.pos };
            points.push(Point { pos, vel: p.vel, rot: p.rot, size: p.size, index: i });
        }
    }
    View { points, strands: Vec::new() }
}

/// An instance placed by a particle: (object index, world matrix).
pub fn instances(scene: &Scene, obj: usize, sys: &ParticleSystem, v: &View) -> Vec<(usize, Mat4)> {
    let s = &sys.settings;
    let pool: Vec<usize> = match s.render_type {
        RenderType::Object => scene.find_object(&s.instance_object).filter(|&i| i != obj).into_iter().collect(),
        RenderType::Collection => match scene.collections.iter().position(|c| c.name == s.instance_collection) {
            Some(ci) => (0..scene.objects.len()).filter(|&i| i != obj && scene.objects[i].collection == ci && matches!(scene.objects[i].data, ObjectData::Mesh(_))).collect(),
            None => Vec::new(),
        },
        _ => Vec::new(),
    };
    if pool.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(v.points.len());
    for p in &v.points {
        let pick = if s.use_collection_pick_random { (rand(sys.seed, p.index as u32, 950) * pool.len() as f32) as usize % pool.len() } else { p.index % pool.len() };
        let o = pool[pick];
        let (_, orot, oscale) = scene.world_matrix(o).to_trs();
        let rot = if s.use_rotations { p.rot } else { Quat::IDENTITY };
        let rot = if s.use_rotation_instance { (rot * orot).normalize() } else { rot };
        let scale = if s.use_scale_instance { oscale * p.size } else { Vec3::splat(p.size) };
        out.push((o, Mat4::from_trs(p.pos, rot, scale)));
    }
    out
}

/// Hair as triangles: a three-sided tube along every strand, tapering from
/// Root to Tip radius times Radius Scale. Returns positions, smooth
/// normals and triangles.
pub fn hair_tubes(strands: &[Vec<Vec3>], s: &ParticleSettings) -> (Vec<Vec3>, Vec<Vec3>, Vec<[u32; 3]>) {
    let mut pos = Vec::new();
    let mut nrm = Vec::new();
    let mut tris = Vec::new();
    for st in strands {
        if st.len() < 2 {
            continue;
        }
        let base = pos.len() as u32;
        let mut prev_side = Vec3::ZERO;
        for (k, &p) in st.iter().enumerate() {
            let t = k as f32 / (st.len() - 1) as f32;
            let r = (s.root_radius + (s.tip_radius - s.root_radius) * t).max(0.0) * s.radius_scale;
            let dir = if k + 1 < st.len() { st[k + 1] - p } else { p - st[k - 1] };
            let dir = if dir.length_squared() > 0.0 { dir.normalize() } else { Vec3::Z };
            let mut side = if prev_side == Vec3::ZERO { dir.any_orthonormal() } else { (prev_side - dir * prev_side.dot(dir)).normalize() };
            if !side.x.is_finite() || side.length_squared() < 0.5 {
                side = dir.any_orthonormal();
            }
            prev_side = side;
            let up = dir.cross(side);
            for j in 0..3 {
                let a = j as f32 / 3.0 * std::f32::consts::TAU;
                let off = side * a.cos() + up * a.sin();
                pos.push(p + off * r.max(1e-5));
                nrm.push(off);
            }
        }
        for k in 0..st.len() as u32 - 1 {
            for j in 0..3u32 {
                let a = base + k * 3 + j;
                let b = base + k * 3 + (j + 1) % 3;
                let c = a + 3;
                let d = b + 3;
                tris.push([a, b, d]);
                tris.push([a, d, c]);
            }
        }
    }
    (pos, nrm, tris)
}
