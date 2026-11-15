// SPDX-License-Identifier: GPL-3.0-only
//! What the simulations add to a render: particle instances (Render As
//! Object / Collection), hair as thin tubes (three-sided prisms, plain
//! triangles for the BVH), and smoke domains as heterogeneous media.
//!
//! Cycles renders neither Halo nor Line particles; neither does this.
//! Simulations are read from the point cache: the caller steps them
//! ([`crate::physics::update`]) before preparing the scene.
use super::bvh::TriGeom;
use super::prepare::TriShade;
use super::volume::{DensityGrid, VolumeCoeffs};
use crate::math::{Mat4, Vec2, Vec3};
use crate::modifiers::Modifier;
use crate::physics::cache::SimFrame;
use crate::physics::{fluid, particles, DomainType, ParticleType, PhysicsModifier, RenderType};
use crate::scene::Scene;

/// A smoke domain's density as a medium: coefficients per unit density
/// and emission per unit flame.
#[derive(Clone, Debug)]
pub(crate) struct GridMedium {
    pub grid: DensityGrid,
    pub sigma_a: Vec3,
    pub sigma_s: Vec3,
    pub flame: Vec3,
}

impl GridMedium {
    pub fn at(&self, p: Vec3) -> VolumeCoeffs {
        let d = self.grid.density_at(p);
        let f = if self.grid.flame.is_some() { self.grid.flame_at(p) } else { 0.0 };
        VolumeCoeffs { sigma_a: self.sigma_a * d, sigma_s: self.sigma_s * d, emission: self.flame * f, anisotropy: 0.0 }
    }

    /// An upper bound of every channel of `sigma_t`.
    pub fn majorant(&self) -> f32 {
        (self.sigma_a + self.sigma_s).max_elem() * self.grid.max_density
    }

    /// Ray parameter range inside the grid's box.
    pub fn clip(&self, o: Vec3, d: Vec3, tmax: f32) -> Option<(f32, f32)> {
        let (lo, hi) = self.grid.bounds();
        let (mut t0, mut t1) = (0.0f32, tmax);
        for a in 0..3 {
            let (oa, da, l, h) = (o.to_array()[a], d.to_array()[a], lo.to_array()[a], hi.to_array()[a]);
            if da.abs() < 1e-12 {
                if oa < l || oa > h {
                    return None;
                }
                continue;
            }
            let (mut n, mut f) = ((l - oa) / da, (h - oa) / da);
            if n > f {
                std::mem::swap(&mut n, &mut f);
            }
            t0 = t0.max(n);
            t1 = t1.min(f);
        }
        (t0 < t1).then_some((t0, t1))
    }
}

/// Principled Volume's smoke defaults (grey scattering smoke) with a
/// blackbody-like flame emission.
fn smoke_medium(grid: DensityGrid) -> GridMedium {
    let color = grid.color.max(Vec3::ZERO).min(Vec3::ONE);
    GridMedium { grid, sigma_s: color, sigma_a: Vec3::ONE - color, flame: Vec3::new(1.0, 0.42, 0.1) * 8.0 }
}

/// The smoke domains at the scene's frame: (object, medium). A domain
/// without cached smoke is an empty medium (still a pass-through box).
pub(crate) fn domains(scene: &Scene) -> Vec<(usize, GridMedium)> {
    let mut out = Vec::new();
    for (oi, slot, s) in fluid::domains(scene) {
        if s.domain_type != DomainType::Gas || !scene.is_visible(oi) {
            continue;
        }
        let key = fluid::key(&scene.objects[oi].name, slot);
        let color = fluid::smoke_color(scene);
        let grid = match scene.physics.caches.frame_at(&key, scene.frame_current.min(s.cache_frame_end)).filter(|_| scene.frame_current >= s.cache_frame_start) {
            Some(f) => match &*f {
                SimFrame::Fluid(ff) => ff.grid(color),
                _ => continue,
            },
            None => DensityGrid::new([1, 1, 1], Vec3::ZERO, 1.0, Default::default(), None, color),
        };
        out.push((oi, smoke_medium(grid)));
    }
    out
}

/// Objects the render leaves out: emitters whose particle settings turn
/// Render > Emitter off.
pub(crate) fn hidden_emitters(scene: &Scene) -> Vec<bool> {
    scene.objects.iter().map(|o| o.modifiers.iter().any(|m| matches!(m, Modifier::Physics(PhysicsModifier::ParticleSystem(p)) if !p.settings.use_render_emitter))).collect()
}

/// Particle instances and hair tubes in world space.
pub(crate) fn particle_triangles(scene: &Scene, default_mat: u32, object_triangles: &dyn Fn(usize, &Mat4) -> (Vec<TriGeom>, Vec<TriShade>)) -> (Vec<TriGeom>, Vec<TriShade>) {
    let mut geoms = Vec::new();
    let mut shades = Vec::new();
    for oi in 0..scene.objects.len() {
        if !scene.is_visible(oi) {
            continue;
        }
        for (slot, _, sys) in particles::systems(scene, oi) {
            let s = &sys.settings;
            let v = particles::view(scene, oi, slot, sys, true);
            let hair = s.kind == ParticleType::Hair || s.render_type == RenderType::Path;
            if hair && !v.strands.is_empty() {
                let (pos, nrm, tris) = particles::hair_tubes(&v.strands, s);
                let slots = &scene.objects[oi].materials;
                let mat = slots.get((s.material_slot.max(1) - 1) as usize).copied().filter(|&m| m < scene.materials.len()).map_or(default_mat, |m| m as u32);
                for t in tris {
                    let p = t.map(|k| pos[k as usize]);
                    let g = TriGeom::new(p[0], p[1], p[2]);
                    if g.e1.cross(g.e2).length_squared() <= 0.0 {
                        continue;
                    }
                    geoms.push(g);
                    shades.push(TriShade { n: t.map(|k| nrm[k as usize]), uv: [Vec2::ZERO; 3], material: mat, light: u32::MAX, object: oi as u32 });
                }
            }
            if matches!(s.render_type, RenderType::Object | RenderType::Collection) {
                for (src, m) in particles::instances(scene, oi, sys, &v) {
                    let (g, sh) = object_triangles(src, &m);
                    geoms.extend(g);
                    shades.extend(sh);
                }
            }
        }
    }
    (geoms, shades)
}
