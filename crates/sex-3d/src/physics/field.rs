// SPDX-License-Identifier: GPL-3.0-only
//! Force fields (Blender's effectors): Force, Wind, Vortex, Magnetic,
//! Harmonic, Charge, Turbulence and Drag with Point or Plane shape and
//! Sphere, Tube or Cone falloff, following `effect.c`'s falloff and
//! force formulas. Particles, cloth, soft bodies, rigid bodies and smoke
//! all sample them through [`Effectors`], scaled by their Field Weights.
use super::settings::{EffectorWeights, FalloffType, FieldSettings, FieldShape, FieldType, ZDirection};
use crate::math::Vec3;
use crate::scene::Scene;

/// One force field in world space.
#[derive(Clone, Debug)]
pub struct Effector {
    pub object: usize,
    pub settings: FieldSettings,
    pub pos: Vec3,
    /// The field's Z axis (wind direction, plane normal, vortex axis).
    pub nor: Vec3,
}

/// The fields and gravity acting on one simulation.
#[derive(Clone, Debug, Default)]
pub struct Effectors {
    pub list: Vec<Effector>,
    pub gravity: Vec3,
    pub weights: EffectorWeights,
}

/// Blender's `falloff_func`.
fn falloff_func(fac: f32, usemin: bool, mindist: f32, usemax: bool, maxdist: f32, power: f32) -> f32 {
    if usemax && fac > maxdist {
        return 0.0;
    }
    if usemin && fac < mindist {
        return 1.0;
    }
    let mindist = if usemin { mindist } else { 0.0 };
    (1.0 + fac - mindist).powf(-power)
}

impl Effector {
    /// (vector to the point, distance, falloff) as `get_effector_data` and
    /// `effector_falloff` compute them.
    pub fn falloff(&self, p: Vec3) -> (Vec3, f32, f32) {
        let s = &self.settings;
        let vec = match s.shape {
            FieldShape::Point => p - self.pos,
            FieldShape::Plane => self.nor * (p - self.pos).dot(self.nor),
        };
        let distance = vec.length();
        let fac = self.nor.dot(vec);
        if (s.z_direction == ZDirection::Positive && fac < 0.0) || (s.z_direction == ZDirection::Negative && fac > 0.0) {
            return (vec, distance, 0.0);
        }
        let dist_f = |d: f32| falloff_func(d, s.use_min_distance, s.distance_min, s.use_max_distance, s.distance_max, s.falloff_power);
        let rad_f = |d: f32| falloff_func(d, s.use_radial_min, s.radial_min, s.use_radial_max, s.radial_max, s.radial_falloff);
        let falloff = match s.falloff_type {
            FalloffType::Sphere => dist_f(distance),
            FalloffType::Tube => {
                let f = dist_f(fac.abs());
                if f == 0.0 {
                    0.0
                } else {
                    let full = p - self.pos;
                    let r = (full - self.nor * full.dot(self.nor)).length();
                    f * rad_f(r)
                }
            }
            FalloffType::Cone => {
                let f = dist_f(fac.abs());
                if f == 0.0 {
                    0.0
                } else {
                    let full = p - self.pos;
                    let l = full.length();
                    let c = if l > 0.0 { (self.nor.dot(full) / l).clamp(-1.0, 1.0) } else { 1.0 };
                    f * rad_f(c.acos().to_degrees())
                }
            }
        };
        (vec, distance, falloff)
    }

    /// The force on a unit point at `p` moving with `vel` (Blender's
    /// `do_physical_effector`), before the field weight.
    pub fn force(&self, p: Vec3, vel: Vec3, time: f32) -> Vec3 {
        let s = &self.settings;
        let (vec, dist, falloff) = self.falloff(p);
        if falloff == 0.0 {
            return Vec3::ZERO;
        }
        let mut strength = s.strength;
        if s.noise > 0.0 {
            let n = noise3(p * 4.0 + Vec3::splat(time * 0.1 + s.seed as f32 * 17.0));
            strength *= 1.0 + s.noise * n.x;
        }
        let unit = |v: Vec3| if v.length_squared() > 0.0 { v.normalize() } else { Vec3::ZERO };
        let mut f = match s.kind {
            FieldType::Force | FieldType::Charge => unit(vec) * (strength * falloff),
            FieldType::Wind => self.nor * (strength * falloff),
            FieldType::Vortex => {
                let full = p - self.pos;
                let tangent = unit(self.nor.cross(full));
                let radial = unit(full - self.nor * full.dot(self.nor));
                tangent * (strength * falloff) - radial * (s.inflow * strength.abs() * falloff)
            }
            FieldType::Magnetic => {
                let t = self.nor.cross(unit(vec)) * (strength * falloff);
                vel.cross(t)
            }
            FieldType::Harmonic => {
                let mut d = vec;
                if s.rest_length > 0.0 && dist > 0.0 {
                    d = d - unit(vec) * s.rest_length;
                }
                -d * (strength * falloff) - vel * s.harmonic_damping
            }
            FieldType::Turbulence => {
                let q = if s.use_global_coords { p } else { p - self.pos };
                let size = if s.size > 1e-6 { s.size } else { 1.0 };
                noise3(q / size + Vec3::splat(s.seed as f32 * 31.7)) * (strength * falloff)
            }
            FieldType::Drag => {
                let speed = vel.length();
                if speed <= 0.0 {
                    Vec3::ZERO
                } else {
                    let lin = s.linear_drag.min(2.0);
                    let quad = s.quadratic_drag.min(2.0);
                    -(vel / speed) * (falloff * speed * (lin + speed * quad))
                }
            }
        };
        if s.flow != 0.0 && matches!(s.kind, FieldType::Force | FieldType::Wind | FieldType::Vortex | FieldType::Turbulence) {
            f -= vel * (s.flow * falloff);
        }
        f
    }
}

impl Effectors {
    /// Every force field in the scene (but `exclude`, the simulated object
    /// itself), with scene gravity, for a simulation with `weights`.
    pub fn collect(scene: &Scene, exclude: Option<usize>, weights: &EffectorWeights) -> Effectors {
        let g = &scene.physics.gravity;
        let gravity = if g.use_gravity { Vec3::from_array(g.gravity) * weights.gravity } else { Vec3::ZERO };
        let coll = if weights.collection.is_empty() { None } else { scene.collections.iter().position(|c| c.name == weights.collection) };
        let mut list = Vec::new();
        for (i, o) in scene.objects.iter().enumerate() {
            let Some(f) = &o.physics.field else { continue };
            if Some(i) == exclude || !scene.is_visible(i) {
                continue;
            }
            if let Some(c) = coll {
                if o.collection != c {
                    continue;
                }
            }
            let m = scene.world_matrix(i);
            let nor = m.transform_vector(Vec3::Z);
            let nor = if nor.length_squared() > 0.0 { nor.normalize() } else { Vec3::Z };
            list.push(Effector { object: i, settings: f.clone(), pos: m.transform_point(Vec3::ZERO), nor });
        }
        Effectors { list, gravity, weights: weights.clone() }
    }

    /// The summed field force at `p` (no gravity), scaled by the weights.
    pub fn force(&self, p: Vec3, vel: Vec3, time: f32) -> Vec3 {
        let mut f = Vec3::ZERO;
        for e in &self.list {
            let w = self.weights.of(e.settings.kind);
            if w == 0.0 || !e.settings.apply_to_location {
                continue;
            }
            f += e.force(p, vel, time) * w;
        }
        f
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

fn hash3(x: i32, y: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

fn grad(h: u32) -> Vec3 {
    let a = (h & 0xffff) as f32 / 65535.0 * std::f32::consts::TAU;
    let z = ((h >> 16) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0;
    let r = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(r * a.cos(), r * a.sin(), z)
}

/// Gradient noise in -1..1, one scalar.
pub fn noise1(p: Vec3, salt: i32) -> f32 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (tx, ty, tz) = (p.x - fx, p.y - fy, p.z - fz);
    let s = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (s(tx), s(ty), s(tz));
    let mut acc = [0.0f32; 8];
    for (k, a) in acc.iter_mut().enumerate() {
        let (dx, dy, dz) = ((k & 1) as i32, ((k >> 1) & 1) as i32, ((k >> 2) & 1) as i32);
        let g = grad(hash3(ix + dx, iy + dy, iz + dz.wrapping_add(salt.wrapping_mul(7919))));
        *a = g.dot(Vec3::new(tx - dx as f32, ty - dy as f32, tz - dz as f32));
    }
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = l(acc[0], acc[1], u);
    let x10 = l(acc[2], acc[3], u);
    let x01 = l(acc[4], acc[5], u);
    let x11 = l(acc[6], acc[7], u);
    (l(l(x00, x10, v), l(x01, x11, v), w) * 1.8).clamp(-1.0, 1.0)
}

/// A noise vector, each channel -1..1.
pub fn noise3(p: Vec3) -> Vec3 {
    Vec3::new(noise1(p, 1), noise1(p, 2), noise1(p, 3))
}
