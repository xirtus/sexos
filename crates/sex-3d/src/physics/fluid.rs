// SPDX-License-Identifier: GPL-3.0-only
//! Fluid (gas): a smoke and fire solver on a MAC grid filling the Domain
//! object's bounds, in place of Blender's Mantaflow.
//!
//! Each frame: Flow objects emit smoke (density, heat, colourless; Fire adds
//! fuel that burns into flame, heat and smoke), Effector objects and the
//! domain border are obstacles, buoyancy (Buoyancy Density and Heat against
//! gravity), vorticity confinement and force fields push the velocity,
//! which is advected semi-Lagrangian and made divergence-free with a
//! pressure solve (conjugate gradients, Jacobi preconditioner); then
//! density, heat, fuel and flame are advected and smoke dissolves.
//!
//! Not done: liquids (FLIP), noise upres, guiding, mesh/particle
//! export, Mantaflow's MacCormack advection. Cached frames keep density
//! and flame (what is drawn and rendered); only the last frame keeps the
//! full state to continue from.
use super::rigid::shape::TriMesh;
use super::settings::{FlowBehavior, FlowType, FluidSettings, FluidType};
use super::PhysicsModifier;
use crate::math::{Aabb, Vec3};
use crate::modifiers::Modifier;
use crate::render::volume::DensityGrid;
use crate::scene::Scene;
use std::sync::Arc;

/// The full solver state.
#[derive(Clone, Debug, Default)]
pub struct FluidState {
    pub density: Vec<f32>,
    pub heat: Vec<f32>,
    pub fuel: Vec<f32>,
    pub flame: Vec<f32>,
    pub u: Vec<f32>,
    pub v: Vec<f32>,
    pub w: Vec<f32>,
}

/// A cached frame of a domain.
#[derive(Clone, Debug)]
pub struct FluidFrame {
    pub res: [usize; 3],
    /// World position of the grid's minimum corner, and the cell size.
    pub min: Vec3,
    pub dx: f32,
    pub density: Arc<Vec<f32>>,
    pub flame: Option<Arc<Vec<f32>>>,
    /// Only on the frame the solver continues from.
    pub state: Option<Box<FluidState>>,
}

impl FluidFrame {
    pub fn bytes(&self) -> usize {
        self.density.len() * 4 + self.flame.as_ref().map_or(0, |f| f.len() * 4) + self.state.as_ref().map_or(0, |s| (s.density.len() * 4 + s.u.len() * 3) * 4)
    }

    /// The grid as the renderer samples it.
    pub fn grid(&self, color: Vec3) -> DensityGrid {
        DensityGrid::new(self.res, self.min, self.dx, self.density.clone(), self.flame.clone(), color)
    }

    pub fn total_density(&self) -> f32 {
        self.density.iter().sum::<f32>() * self.dx.powi(3)
    }
}

pub fn key(obj: &str, slot: usize) -> String {
    format!("{obj}/fluid{slot}")
}

/// The domains in the scene: (object, modifier slot, settings).
pub fn domains(scene: &Scene) -> Vec<(usize, usize, &FluidSettings)> {
    let mut out = Vec::new();
    for (i, o) in scene.objects.iter().enumerate() {
        for (k, m) in o.modifiers.iter().enumerate() {
            if let Modifier::Physics(PhysicsModifier::Fluid(f)) = m {
                if f.fluid_type == FluidType::Domain {
                    out.push((i, k, &**f));
                }
            }
        }
    }
    out
}

pub(super) struct Grid {
    pub(super) n: [usize; 3],
    pub(super) min: Vec3,
    pub(super) dx: f32,
}

impl Grid {
    pub(super) fn cells(&self) -> usize {
        self.n[0] * self.n[1] * self.n[2]
    }
    pub(super) fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.n[0] * (j + self.n[1] * k)
    }
    pub(super) fn center(&self, i: usize, j: usize, k: usize) -> Vec3 {
        self.min + Vec3::new(i as f32 + 0.5, j as f32 + 0.5, k as f32 + 0.5) * self.dx
    }
    /// Size of the staggered grid for axis `a`.
    fn face_n(&self, a: usize) -> [usize; 3] {
        let mut n = self.n;
        n[a] += 1;
        n
    }
}

fn domain_grid(scene: &Scene, obj: usize, s: &FluidSettings) -> Option<Grid> {
    let (m, _) = scene.world_mesh(obj)?;
    let b = m.bounds();
    if b.is_empty() {
        return None;
    }
    let size = b.size();
    let longest = size.max_elem().max(1e-4);
    let res = s.resolution_max.clamp(4, 256) as f32;
    let dx = longest / res;
    let n = [(size.x / dx).round().max(1.0) as usize, (size.y / dx).round().max(1.0) as usize, (size.z / dx).round().max(1.0) as usize];
    Some(Grid { n, min: b.min, dx })
}

/// Trilinear sample of a grid of size `n` whose samples sit at
/// `(i + off.x, j + off.y, k + off.z)` in cell units.
pub(super) fn sample(q: &[f32], n: [usize; 3], p: Vec3, off: Vec3) -> f32 {
    let x = (p.x - off.x).clamp(0.0, (n[0] - 1) as f32);
    let y = (p.y - off.y).clamp(0.0, (n[1] - 1) as f32);
    let z = (p.z - off.z).clamp(0.0, (n[2] - 1) as f32);
    let (i, j, k) = (x.floor() as usize, y.floor() as usize, z.floor() as usize);
    let (i1, j1, k1) = ((i + 1).min(n[0] - 1), (j + 1).min(n[1] - 1), (k + 1).min(n[2] - 1));
    let (fx, fy, fz) = (x - i as f32, y - j as f32, z - k as f32);
    let at = |a: usize, b: usize, c: usize| q[a + n[0] * (b + n[1] * c)];
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    l(
        l(l(at(i, j, k), at(i1, j, k), fx), l(at(i, j1, k), at(i1, j1, k), fx), fy),
        l(l(at(i, j, k1), at(i1, j, k1), fx), l(at(i, j1, k1), at(i1, j1, k1), fx), fy),
        fz,
    )
}

struct Solver<'a> {
    g: Grid,
    st: FluidState,
    solid: Vec<bool>,
    s: &'a FluidSettings,
}

impl Solver<'_> {
    /// Velocity at a point in cell units (cell centres at +0.5).
    fn vel(&self, p: Vec3) -> Vec3 {
        let g = &self.g;
        Vec3::new(
            sample(&self.st.u, g.face_n(0), p, Vec3::new(0.0, 0.5, 0.5)),
            sample(&self.st.v, g.face_n(1), p, Vec3::new(0.5, 0.0, 0.5)),
            sample(&self.st.w, g.face_n(2), p, Vec3::new(0.5, 0.5, 0.0)),
        )
    }

    fn cell_vel(&self, i: usize, j: usize, k: usize) -> Vec3 {
        let g = &self.g;
        let u = |a, b, c| self.st.u[a + (g.n[0] + 1) * (b + g.n[1] * c)];
        let v = |a, b, c| self.st.v[a + g.n[0] * (b + (g.n[1] + 1) * c)];
        let w = |a, b, c| self.st.w[a + g.n[0] * (b + g.n[1] * c)];
        Vec3::new((u(i, j, k) + u(i + 1, j, k)) * 0.5, (v(i, j, k) + v(i, j + 1, k)) * 0.5, (w(i, j, k) + w(i, j, k + 1)) * 0.5)
    }

    fn max_speed(&self) -> f32 {
        let m = |q: &[f32]| q.iter().fold(0.0f32, |a, &x| a.max(x.abs()));
        m(&self.st.u).max(m(&self.st.v)).max(m(&self.st.w))
    }

    fn advect_scalar(&self, q: &[f32], dt: f32) -> Vec<f32> {
        let g = &self.g;
        let mut out = vec![0.0; q.len()];
        for k in 0..g.n[2] {
            for j in 0..g.n[1] {
                for i in 0..g.n[0] {
                    let id = g.idx(i, j, k);
                    if self.solid[id] {
                        continue;
                    }
                    let p = Vec3::new(i as f32 + 0.5, j as f32 + 0.5, k as f32 + 0.5);
                    let v = self.cell_vel(i, j, k);
                    let mid = p - v * (0.5 * dt / g.dx);
                    let back = p - self.vel(mid) * (dt / g.dx);
                    out[id] = sample(q, g.n, back, Vec3::splat(0.5));
                }
            }
        }
        out
    }

    fn advect_velocity(&mut self, dt: f32) {
        let g = &self.g;
        let mut nu = self.st.u.clone();
        let mut nv = self.st.v.clone();
        let mut nw = self.st.w.clone();
        for (a, out) in [(0usize, &mut nu), (1, &mut nv), (2, &mut nw)] {
            let fnn = g.face_n(a);
            for k in 0..fnn[2] {
                for j in 0..fnn[1] {
                    for i in 0..fnn[0] {
                        let mut p = Vec3::new(i as f32 + 0.5, j as f32 + 0.5, k as f32 + 0.5);
                        match a {
                            0 => p.x -= 0.5,
                            1 => p.y -= 0.5,
                            _ => p.z -= 0.5,
                        }
                        let back = p - self.vel(p) * (dt / g.dx);
                        let v = self.vel(back);
                        out[i + fnn[0] * (j + fnn[1] * k)] = [v.x, v.y, v.z][a];
                    }
                }
            }
        }
        self.st.u = nu;
        self.st.v = nv;
        self.st.w = nw;
    }

    fn add_cell_force(&mut self, f: &[Vec3], dt: f32) {
        let g = &self.g;
        let n = g.n;
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let a = f[g.idx(i, j, k)] * (0.5 * dt);
                    if a == Vec3::ZERO {
                        continue;
                    }
                    let ui = |x: usize| x + (n[0] + 1) * (j + n[1] * k);
                    self.st.u[ui(i)] += a.x;
                    self.st.u[ui(i + 1)] += a.x;
                    let vi = |y: usize| i + n[0] * (y + (n[1] + 1) * k);
                    self.st.v[vi(j)] += a.y;
                    self.st.v[vi(j + 1)] += a.y;
                    let wi = |z: usize| i + n[0] * (j + n[1] * z);
                    self.st.w[wi(k)] += a.z;
                    self.st.w[wi(k + 1)] += a.z;
                }
            }
        }
    }

    fn forces(&mut self, dt: f32, gravity: Vec3, fields: &super::field::Effectors, t: f32) {
        let g = &self.g;
        let n = g.n;
        let cells = g.cells();
        let mut f = vec![Vec3::ZERO; cells];
        // Buoyancy: smoke and heat rise against gravity.
        let up = -gravity * 0.1;
        for id in 0..cells {
            if self.solid[id] {
                continue;
            }
            f[id] = up * (self.s.alpha * self.st.density[id] + self.s.beta * self.st.heat[id]);
        }
        // Vorticity confinement.
        let eps = self.s.vorticity + if self.st.flame.iter().any(|&x| x > 0.0) { self.s.flame_vorticity } else { 0.0 };
        if eps > 0.0 && n.iter().all(|&x| x >= 3) {
            let mut vc = vec![Vec3::ZERO; cells];
            for k in 0..n[2] {
                for j in 0..n[1] {
                    for i in 0..n[0] {
                        vc[g.idx(i, j, k)] = self.cell_vel(i, j, k);
                    }
                }
            }
            let mut omega = vec![Vec3::ZERO; cells];
            for k in 1..n[2] - 1 {
                for j in 1..n[1] - 1 {
                    for i in 1..n[0] - 1 {
                        let d = |di: isize, dj: isize, dk: isize| vc[g.idx((i as isize + di) as usize, (j as isize + dj) as usize, (k as isize + dk) as usize)];
                        let h = 0.5 / g.dx;
                        let dwdy = (d(0, 1, 0).z - d(0, -1, 0).z) * h;
                        let dvdz = (d(0, 0, 1).y - d(0, 0, -1).y) * h;
                        let dudz = (d(0, 0, 1).x - d(0, 0, -1).x) * h;
                        let dwdx = (d(1, 0, 0).z - d(-1, 0, 0).z) * h;
                        let dvdx = (d(1, 0, 0).y - d(-1, 0, 0).y) * h;
                        let dudy = (d(0, 1, 0).x - d(0, -1, 0).x) * h;
                        omega[g.idx(i, j, k)] = Vec3::new(dwdy - dvdz, dudz - dwdx, dvdx - dudy);
                    }
                }
            }
            for k in 1..n[2] - 1 {
                for j in 1..n[1] - 1 {
                    for i in 1..n[0] - 1 {
                        let m = |di: isize, dj: isize, dk: isize| omega[g.idx((i as isize + di) as usize, (j as isize + dj) as usize, (k as isize + dk) as usize)].length();
                        let grad = Vec3::new(m(1, 0, 0) - m(-1, 0, 0), m(0, 1, 0) - m(0, -1, 0), m(0, 0, 1) - m(0, 0, -1));
                        let l = grad.length();
                        if l < 1e-9 {
                            continue;
                        }
                        let nn = grad / l;
                        let id = g.idx(i, j, k);
                        f[id] += nn.cross(omega[id]) * (eps * g.dx * 2.0);
                    }
                }
            }
        }
        // Force fields.
        if !fields.is_empty() {
            for k in 0..n[2] {
                for j in 0..n[1] {
                    for i in 0..n[0] {
                        let id = g.idx(i, j, k);
                        if self.st.density[id] > 1e-4 || self.st.heat[id].abs() > 1e-4 {
                            f[id] += fields.force(g.center(i, j, k), self.cell_vel(i, j, k), t);
                        }
                    }
                }
            }
        }
        self.add_cell_force(&f, dt);
    }

    fn enforce_solid(&mut self) {
        let g = &self.g;
        let n = g.n;
        let border = self.s.use_collision_border;
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..=n[0] {
                    let a = if i == 0 { None } else { Some(g.idx(i - 1, j, k)) };
                    let b = if i == n[0] { None } else { Some(g.idx(i, j, k)) };
                    if (a.is_none() || b.is_none()) && border || a.is_some_and(|x| self.solid[x]) || b.is_some_and(|x| self.solid[x]) {
                        self.st.u[i + (n[0] + 1) * (j + n[1] * k)] = 0.0;
                    }
                }
            }
        }
        for k in 0..n[2] {
            for j in 0..=n[1] {
                for i in 0..n[0] {
                    let a = if j == 0 { None } else { Some(g.idx(i, j - 1, k)) };
                    let b = if j == n[1] { None } else { Some(g.idx(i, j, k)) };
                    if (a.is_none() || b.is_none()) && border || a.is_some_and(|x| self.solid[x]) || b.is_some_and(|x| self.solid[x]) {
                        self.st.v[i + n[0] * (j + (n[1] + 1) * k)] = 0.0;
                    }
                }
            }
        }
        for k in 0..=n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let a = if k == 0 { None } else { Some(g.idx(i, j, k - 1)) };
                    let b = if k == n[2] { None } else { Some(g.idx(i, j, k)) };
                    if (a.is_none() || b.is_none()) && border || a.is_some_and(|x| self.solid[x]) || b.is_some_and(|x| self.solid[x]) {
                        self.st.w[i + n[0] * (j + n[1] * k)] = 0.0;
                    }
                }
            }
        }
    }

    /// Make the velocity divergence-free.
    fn project(&mut self) {
        self.enforce_solid();
        let g = &self.g;
        let n = g.n;
        let cells = g.cells();
        let border = self.s.use_collision_border;
        let ui = |i: usize, j: usize, k: usize| i + (n[0] + 1) * (j + n[1] * k);
        let vi = |i: usize, j: usize, k: usize| i + n[0] * (j + (n[1] + 1) * k);
        let wi = |i: usize, j: usize, k: usize| i + n[0] * (j + n[1] * k);
        let mut b = vec![0.0f32; cells];
        let mut diag = vec![0.0f32; cells];
        // Neighbours (or usize::MAX for none/solid) per cell.
        let mut nb = vec![[usize::MAX; 6]; cells];
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let id = g.idx(i, j, k);
                    if self.solid[id] {
                        continue;
                    }
                    let div = self.st.u[ui(i + 1, j, k)] - self.st.u[ui(i, j, k)] + self.st.v[vi(i, j + 1, k)] - self.st.v[vi(i, j, k)] + self.st.w[wi(i, j, k + 1)] - self.st.w[wi(i, j, k)];
                    b[id] = -div * g.dx;
                    let cand = [
                        (i > 0).then(|| g.idx(i.wrapping_sub(1), j, k)),
                        (i + 1 < n[0]).then(|| g.idx(i + 1, j, k)),
                        (j > 0).then(|| g.idx(i, j.wrapping_sub(1), k)),
                        (j + 1 < n[1]).then(|| g.idx(i, j + 1, k)),
                        (k > 0).then(|| g.idx(i, j, k.wrapping_sub(1))),
                        (k + 1 < n[2]).then(|| g.idx(i, j, k + 1)),
                    ];
                    for (s, c) in cand.iter().enumerate() {
                        match c {
                            Some(c) if !self.solid[*c] => {
                                diag[id] += 1.0;
                                nb[id][s] = *c;
                            }
                            Some(_) => {}
                            None => {
                                if !border {
                                    diag[id] += 1.0; // open: pressure 0 outside
                                }
                            }
                        }
                    }
                }
            }
        }
        // Conjugate gradients with a Jacobi preconditioner.
        let apply = |x: &[f32], out: &mut [f32]| {
            for id in 0..cells {
                if diag[id] == 0.0 {
                    out[id] = 0.0;
                    continue;
                }
                let mut s = diag[id] * x[id];
                for &c in &nb[id] {
                    if c != usize::MAX {
                        s -= x[c];
                    }
                }
                out[id] = s;
            }
        };
        let mut x = vec![0.0f32; cells];
        let mut r = b.clone();
        // Closed domains: remove the mean so the system is consistent.
        if border {
            let fluid: Vec<usize> = (0..cells).filter(|&i| diag[i] > 0.0).collect();
            if !fluid.is_empty() {
                let mean = fluid.iter().map(|&i| r[i]).sum::<f32>() / fluid.len() as f32;
                for &i in &fluid {
                    r[i] -= mean;
                }
            }
        }
        let pre = |r: &[f32], z: &mut [f32]| {
            for i in 0..cells {
                z[i] = if diag[i] > 0.0 { r[i] / diag[i] } else { 0.0 };
            }
        };
        let mut z = vec![0.0f32; cells];
        pre(&r, &mut z);
        let mut p = z.clone();
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| *a as f64 * *b as f64).sum();
        let r0: f64 = r.iter().map(|a| (*a as f64).powi(2)).sum::<f64>().sqrt();
        let mut ap = vec![0.0f32; cells];
        if r0 > 1e-12 {
            for _ in 0..300 {
                apply(&p, &mut ap);
                let pap: f64 = p.iter().zip(&ap).map(|(a, b)| *a as f64 * *b as f64).sum();
                if pap.abs() < 1e-30 {
                    break;
                }
                let alpha = (rz / pap) as f32;
                for i in 0..cells {
                    x[i] += alpha * p[i];
                    r[i] -= alpha * ap[i];
                }
                let rn: f64 = r.iter().map(|a| (*a as f64).powi(2)).sum::<f64>().sqrt();
                if rn < 1e-5 * r0 {
                    break;
                }
                pre(&r, &mut z);
                let rz2: f64 = r.iter().zip(&z).map(|(a, b)| *a as f64 * *b as f64).sum();
                let beta = (rz2 / rz) as f32;
                rz = rz2;
                for i in 0..cells {
                    p[i] = z[i] + beta * p[i];
                }
            }
        }
        // Subtract the pressure gradient.
        let solid = |c: Option<usize>| c.is_none_or(|c| self.solid[c]);
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    let id = g.idx(i, j, k);
                    if self.solid[id] {
                        continue;
                    }
                    if i + 1 < n[0] && !solid(Some(g.idx(i + 1, j, k))) {
                        self.st.u[ui(i + 1, j, k)] -= (x[g.idx(i + 1, j, k)] - x[id]) / g.dx;
                    } else if i + 1 == n[0] && !border {
                        self.st.u[ui(i + 1, j, k)] -= (0.0 - x[id]) / g.dx;
                    }
                    if i == 0 && !border {
                        self.st.u[ui(0, j, k)] -= (x[id] - 0.0) / g.dx;
                    }
                    if j + 1 < n[1] && !solid(Some(g.idx(i, j + 1, k))) {
                        self.st.v[vi(i, j + 1, k)] -= (x[g.idx(i, j + 1, k)] - x[id]) / g.dx;
                    } else if j + 1 == n[1] && !border {
                        self.st.v[vi(i, j + 1, k)] -= (0.0 - x[id]) / g.dx;
                    }
                    if j == 0 && !border {
                        self.st.v[vi(i, 0, k)] -= (x[id] - 0.0) / g.dx;
                    }
                    if k + 1 < n[2] && !solid(Some(g.idx(i, j, k + 1))) {
                        self.st.w[wi(i, j, k + 1)] -= (x[g.idx(i, j, k + 1)] - x[id]) / g.dx;
                    } else if k + 1 == n[2] && !border {
                        self.st.w[wi(i, j, k + 1)] -= (0.0 - x[id]) / g.dx;
                    }
                    if k == 0 && !border {
                        self.st.w[wi(i, j, 0)] -= (x[id] - 0.0) / g.dx;
                    }
                }
            }
        }
        self.enforce_solid();
    }

    /// Largest divergence magnitude (tests).
    fn max_divergence(&self) -> f32 {
        let g = &self.g;
        let n = g.n;
        let mut m = 0.0f32;
        for k in 0..n[2] {
            for j in 0..n[1] {
                for i in 0..n[0] {
                    if self.solid[g.idx(i, j, k)] {
                        continue;
                    }
                    let d = self.st.u[i + 1 + (n[0] + 1) * (j + n[1] * k)] - self.st.u[i + (n[0] + 1) * (j + n[1] * k)] + self.st.v[i + n[0] * (j + 1 + (n[1] + 1) * k)] - self.st.v[i + n[0] * (j + (n[1] + 1) * k)]
                        + self.st.w[i + n[0] * (j + n[1] * (k + 1))]
                        - self.st.w[i + n[0] * (j + n[1] * k)];
                    m = m.max(d.abs());
                }
            }
        }
        m / g.dx
    }
}

/// A mesh rasterised onto the grid: per cell 0 (outside), 1 (surface band)
/// or `volume` (inside).
pub(super) fn rasterize(g: &Grid, mesh: &TriMesh, band: f32, volume: f32, plane: bool) -> Vec<f32> {
    let mut out = vec![0.0f32; g.cells()];
    let b = mesh.bounds();
    if b.is_empty() {
        return out;
    }
    let lo = |x: f32, a: usize| (((x - band - g.min[a]) / g.dx).floor().max(0.0) as usize).min(g.n[a]);
    let hi = |x: f32, a: usize| (((x + band - g.min[a]) / g.dx).ceil().max(0.0) as usize).min(g.n[a]);
    let mut scratch = Vec::new();
    let tris: Vec<[Vec3; 3]> = (0..mesh.tris.len()).map(|t| mesh.tri(t)).collect();
    for k in lo(b.min.z, 2)..hi(b.max.z, 2) {
        for j in lo(b.min.y, 1)..hi(b.max.y, 1) {
            for i in lo(b.min.x, 0)..hi(b.max.x, 0) {
                let p = g.center(i, j, k);
                // Surface band.
                scratch.clear();
                mesh.query(&Aabb { min: p - Vec3::splat(band), max: p + Vec3::splat(band) }, &mut scratch);
                let near = scratch.iter().any(|&t| {
                    let [a, bb, c] = mesh.tri(t);
                    super::collision::closest_on_triangle(p, a, bb, c).distance(p) <= band
                });
                let mut val = if near { 1.0f32 } else { 0.0 };
                if !plane && volume > 0.0 && !near {
                    // Inside test by parity along +X.
                    let mut hits = 0;
                    for [a, bb, c] in &tris {
                        let d = Vec3::new(1.0, 0.000_37, 0.000_19);
                        let e1 = *bb - *a;
                        let e2 = *c - *a;
                        let h = d.cross(e2);
                        let det = e1.dot(h);
                        if det.abs() < 1e-12 {
                            continue;
                        }
                        let inv = 1.0 / det;
                        let s = p - *a;
                        let u = s.dot(h) * inv;
                        if !(0.0..=1.0).contains(&u) {
                            continue;
                        }
                        let q = s.cross(e1);
                        let v = d.dot(q) * inv;
                        if v < 0.0 || u + v > 1.0 {
                            continue;
                        }
                        if e2.dot(q) * inv > 0.0 {
                            hits += 1;
                        }
                    }
                    if hits % 2 == 1 {
                        val = val.max(volume);
                    }
                }
                out[g.idx(i, j, k)] = val;
            }
        }
    }
    out
}

pub(super) fn world_trimesh(scene: &Scene, i: usize) -> Option<TriMesh> {
    let (m, _) = scene.world_mesh(i)?;
    let tris: Vec<[u32; 3]> = m.triangulate().iter().map(|t| t.verts).collect();
    if tris.is_empty() {
        return None;
    }
    Some(TriMesh::new(m.positions, tris))
}

/// An empty domain at the start frame.
pub fn initial(scene: &Scene, obj: usize, s: &FluidSettings) -> Option<FluidFrame> {
    let g = domain_grid(scene, obj, s)?;
    let c = g.cells();
    let st = FluidState {
        density: vec![0.0; c],
        heat: vec![0.0; c],
        fuel: vec![0.0; c],
        flame: vec![0.0; c],
        u: vec![0.0; (g.n[0] + 1) * g.n[1] * g.n[2]],
        v: vec![0.0; g.n[0] * (g.n[1] + 1) * g.n[2]],
        w: vec![0.0; g.n[0] * g.n[1] * (g.n[2] + 1)],
    };
    let mut f = FluidFrame { res: g.n, min: g.min, dx: g.dx, density: Arc::new(st.density.clone()), flame: None, state: Some(Box::new(st)) };
    // Emit at the start frame too (Geometry flows only emit here).
    f = step_inner(scene, obj, s, &f, true)?;
    Some(f)
}

/// Advance a domain one frame.
pub fn step(scene: &Scene, obj: usize, s: &FluidSettings, prev: &FluidFrame) -> Option<FluidFrame> {
    step_inner(scene, obj, s, prev, false)
}

fn step_inner(scene: &Scene, obj: usize, s: &FluidSettings, prev: &FluidFrame, first: bool) -> Option<FluidFrame> {
    let st = prev.state.as_ref()?.as_ref().clone();
    let g = Grid { n: prev.res, min: prev.min, dx: prev.dx };
    let cells = g.cells();
    let mut solver = Solver { g, st, solid: vec![false; cells], s };
    // Obstacles.
    for (i, o) in scene.objects.iter().enumerate() {
        for m in &o.modifiers {
            if let Modifier::Physics(PhysicsModifier::Fluid(f)) = m {
                if f.fluid_type == FluidType::Effector && f.use_effector && i != obj {
                    if let Some(tm) = world_trimesh(scene, i) {
                        let mask = rasterize(&solver.g, &tm, (f.effector_surface_distance * solver.g.dx).max(solver.g.dx * 0.5), 1.0, f.use_plane_init);
                        for (sd, &mk) in solver.solid.iter_mut().zip(&mask) {
                            if mk > 0.0 {
                                *sd = true;
                            }
                        }
                    }
                }
            }
        }
    }
    // Flows (rasterised once per frame).
    struct Src {
        mask: Vec<f32>,
        s: FluidSettings,
        vel: Vec3,
    }
    let mut srcs = Vec::new();
    for (i, o) in scene.objects.iter().enumerate() {
        for m in &o.modifiers {
            if let Modifier::Physics(PhysicsModifier::Fluid(f)) = m {
                if f.fluid_type != FluidType::Flow || !f.use_flow || i == obj || f.flow_type == FlowType::Liquid {
                    continue;
                }
                if f.flow_behavior == FlowBehavior::Geometry && !first {
                    continue;
                }
                let Some(tm) = world_trimesh(scene, i) else { continue };
                let band = (f.surface_distance * solver.g.dx).max(solver.g.dx * 0.5);
                let mask = rasterize(&solver.g, &tm, band, f.volume_density, f.use_plane_init);
                let vel = Vec3::from_array(f.velocity_coord);
                srcs.push(Src { mask, s: (**f).clone(), vel });
            }
        }
    }
    let gs = &scene.physics.gravity;
    let gravity = if gs.use_gravity { Vec3::from_array(gs.gravity) * s.effector_weights.gravity } else { Vec3::ZERO };
    let fields = super::field::Effectors::collect(scene, Some(obj), &s.effector_weights);
    let dt_frame = s.time_scale / scene.fps.max(1e-3);
    let steps = if first {
        0
    } else if s.use_adaptive_timesteps {
        let need = (solver.max_speed() * dt_frame / (s.cfl_condition.max(0.1) * solver.g.dx)).ceil() as i32;
        need.clamp(s.timesteps_min.max(1), s.timesteps_max.max(1)) as usize
    } else {
        s.timesteps_min.max(1) as usize
    };
    let emit = |solver: &mut Solver, srcs: &[Src]| {
        for src in srcs {
            let f = &src.s;
            for id in 0..cells {
                let m = src.mask[id];
                if m <= 0.0 || solver.solid[id] {
                    continue;
                }
                match f.flow_behavior {
                    FlowBehavior::Outflow => {
                        solver.st.density[id] = 0.0;
                        solver.st.heat[id] = 0.0;
                        solver.st.fuel[id] = 0.0;
                        solver.st.flame[id] = 0.0;
                    }
                    _ => {
                        if matches!(f.flow_type, FlowType::Smoke | FlowType::Both) {
                            solver.st.density[id] = solver.st.density[id].max(f.density * m);
                        }
                        if matches!(f.flow_type, FlowType::Fire | FlowType::Both) {
                            solver.st.fuel[id] = solver.st.fuel[id].max(f.fuel_amount * m);
                        }
                        if f.temperature > 0.0 {
                            solver.st.heat[id] = solver.st.heat[id].max(f.temperature * m);
                        } else {
                            solver.st.heat[id] = solver.st.heat[id].min(f.temperature * m);
                        }
                    }
                }
            }
            if f.use_initial_velocity && src.vel != Vec3::ZERO {
                let mut force = vec![Vec3::ZERO; cells];
                for id in 0..cells {
                    if src.mask[id] > 0.0 {
                        force[id] = src.vel * f.velocity_factor;
                    }
                }
                // Set (not add) the initial velocity: a one-step impulse towards it.
                solver.add_cell_force(&force, 1.0 / steps.max(1) as f32);
            }
        }
    };
    if first {
        emit(&mut solver, &srcs);
    }
    let dt = if steps > 0 { dt_frame / steps as f32 } else { 0.0 };
    for _ in 0..steps {
        emit(&mut solver, &srcs);
        solver.forces(dt, gravity, &fields, scene.frame_current as f32);
        solver.project();
        solver.advect_velocity(dt);
        solver.project();
        solver.st.density = solver.advect_scalar(&solver.st.density, dt);
        solver.st.heat = solver.advect_scalar(&solver.st.heat, dt);
        if solver.st.fuel.iter().any(|&x| x > 0.0) || solver.st.flame.iter().any(|&x| x > 0.0) {
            solver.st.fuel = solver.advect_scalar(&solver.st.fuel, dt);
            // Burning: fuel turns into flame, heat and smoke.
            for id in 0..cells {
                let burn = solver.st.fuel[id] * (s.burning_rate * dt * 4.0).min(1.0);
                solver.st.fuel[id] -= burn;
                solver.st.flame[id] = (solver.st.flame[id] * 0.5 + burn / dt.max(1e-6) * 0.25).min(1.0);
                solver.st.heat[id] += burn * s.flame_max_temp;
                solver.st.density[id] += burn * s.flame_smoke * 0.25;
            }
        }
    }
    if !first && s.use_dissolve_smoke && s.dissolve_speed > 0.0 {
        let k = 1.0 / s.dissolve_speed;
        for d in solver.st.density.iter_mut() {
            if s.use_dissolve_smoke_log {
                *d *= 1.0 - k;
            } else {
                *d = (*d - k).max(0.0);
            }
        }
    }
    for (d, &sd) in solver.st.density.iter_mut().zip(&solver.solid) {
        if sd {
            *d = 0.0;
        }
    }
    let flame = solver.st.flame.iter().any(|&x| x > 0.0).then(|| Arc::new(solver.st.flame.clone()));
    let st = solver.st;
    Some(FluidFrame { res: prev.res, min: prev.min, dx: prev.dx, density: Arc::new(st.density.clone()), flame, state: Some(Box::new(st)) })
}

/// The divergence left after projection, for tests.
pub fn test_divergence(f: &FluidFrame, s: &FluidSettings) -> f32 {
    let Some(st) = &f.state else { return 0.0 };
    let g = Grid { n: f.res, min: f.min, dx: f.dx };
    let cells = g.cells();
    let solver = Solver { g, st: (**st).clone(), solid: vec![false; cells], s };
    solver.max_divergence()
}

/// A frame for the cache: drop the full state.
pub fn display_copy(f: &FluidFrame) -> FluidFrame {
    FluidFrame { res: f.res, min: f.min, dx: f.dx, density: f.density.clone(), flame: f.flame.clone(), state: None }
}

/// Domain's smoke colour for rendering: the colour of its flows.
pub fn smoke_color(scene: &Scene) -> Vec3 {
    for o in &scene.objects {
        for m in &o.modifiers {
            if let Modifier::Physics(PhysicsModifier::Fluid(f)) = m {
                if f.fluid_type == FluidType::Flow {
                    return Vec3::from_array(f.smoke_color);
                }
            }
        }
    }
    Vec3::splat(0.7)
}
