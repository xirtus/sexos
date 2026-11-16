// SPDX-License-Identifier: GPL-3.0-only
//! Fluid (liquid): a FLIP/APIC liquid solver on the Domain's grid, in place
//! of Blender's Mantaflow liquid, plus the liquid surface mesh.
//!
//! The liquid is a set of particles (Blender's "Sampling" per cell and
//! axis, jittered by "Randomness") carrying velocity. Each substep:
//!
//! 1. Flow objects with Flow Type Liquid emit: Geometry fills its volume at
//!    the start frame, Inflow refills empty cells inside it every substep,
//!    Outflow deletes the particles inside it.
//! 2. Particle velocities are transferred to the staggered (MAC) grid with
//!    trilinear weights (APIC adds each particle's affine velocity).
//! 3. Gravity (the scene's, or the domain's when the scene has none, times
//!    Field Weights > Gravity) and force fields are added; Diffusion's
//!    viscosity (Base × 10^-Exponent, plus the High Viscosity Solver's
//!    Strength) is an implicit diffusion of the velocity.
//! 4. Pressure makes the velocity divergence-free: liquid cells are the
//!    unknowns, air cells have zero pressure (or the surface tension jump
//!    σκ, ghost-fluid style), Effector obstacles and closed Border
//!    Collisions sides are walls, open sides are air.
//! 5. Velocities go back to the particles: FLIP blends the grid's change
//!    into each particle's own velocity (FLIP Ratio; 0 is PIC), APIC takes
//!    the grid velocity and its gradient.
//! 6. Particles move through the grid velocity (RK2, sub-cycled to at most
//!    one cell per move) and are stopped by walls and obstacles; particles
//!    leaving through open sides are deleted.
//!
//! After each move a volume correction evens out the particles: cells
//! packed more densely than they were sampled push particles out, interior
//! cells thinned out pull them in (a Poisson solve on the density error
//! that moves positions only). The particle count stays constant in a
//! closed domain and the liquid keeps its volume: nothing is deleted for
//! crowding or reseeded for thinning (both would change the volume).
//!
//! The surface: each particle splats its volume onto a finer grid (Mesh >
//! Upres Factor), with a tent kernel as wide as Mesh > Particle Radius
//! particle spacings; the 0.5 iso-surface of that volume fraction is the
//! liquid's surface (for evenly spread particles it encloses exactly their
//! volume). Smoothing Positive / Negative relax the field before marching
//! cubes extracts a closed, outward-facing triangle mesh. "Preview" skips
//! the upres and the smoothing.
//!
//! Everything runs deterministically: parallel parts (transfers, particle
//! moves, linear solves, smoothing) compute each value independently or
//! sum in fixed blocks, so any thread count gives identical results.
//!
//! Not done: Mantaflow's narrow-band FLIP (the whole volume holds
//! particles; Narrow Band Width is kept), fractional obstacles, moving
//! obstacles pushing liquid (they only block it), secondary particles
//! (spray, foam, bubbles), guides, Flow Source Particle System, Sampling
//! Substeps, Particles Maximum / Minimum and Concavity (kept, not used).
use super::fluid::{rasterize, sample, world_trimesh, Grid};
use super::settings::{FlowBehavior, FlowSource, FlowType, FluidSettings, FluidType, MeshGenerator, SimulationMethod};
use super::PhysicsModifier;
use crate::math::Vec3;
use crate::mesh::{Face, Mesh};
use crate::modifiers::Modifier;
use crate::scene::{ObjectData, Scene};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------
// Threads

static THREADS: AtomicUsize = AtomicUsize::new(0);

/// Worker threads for the liquid solver (0: one per core). The results
/// never depend on it.
pub fn set_threads(n: usize) {
    THREADS.store(n, Ordering::Relaxed);
}

pub fn threads() -> usize {
    match THREADS.load(Ordering::Relaxed) {
        0 => std::thread::available_parallelism().map_or(1, |n| n.get()).min(16),
        n => n,
    }
}

/// Below this many items a loop runs on the calling thread.
const PAR_MIN: usize = 8192;

/// `f(i, &mut out[i])` for every item, split over the threads.
fn par_for<T: Send>(out: &mut [T], f: impl Fn(usize, &mut T) + Sync) {
    let t = threads();
    if t <= 1 || out.len() < PAR_MIN {
        for (i, x) in out.iter_mut().enumerate() {
            f(i, x);
        }
        return;
    }
    let chunk = out.len().div_ceil(t);
    let f = &f;
    std::thread::scope(|s| {
        for (c, part) in out.chunks_mut(chunk).enumerate() {
            s.spawn(move || {
                for (k, x) in part.iter_mut().enumerate() {
                    f(c * chunk + k, x);
                }
            });
        }
    });
}

/// Sum in fixed blocks (independent of the thread count).
fn dot(a: &[f32], b: &[f32]) -> f64 {
    const BLOCK: usize = 4096;
    let mut parts = vec![0.0f64; a.len().div_ceil(BLOCK)];
    par_for(&mut parts, |bi, out| {
        let (s, e) = (bi * BLOCK, ((bi + 1) * BLOCK).min(a.len()));
        *out = (s..e).map(|i| a[i] as f64 * b[i] as f64).sum();
    });
    parts.iter().sum()
}

// ---------------------------------------------------------------------------
// Frames

/// The solver state the next frame continues from.
#[derive(Clone, Debug, Default)]
pub struct LiquidState {
    /// Particle positions in grid units (cells from the grid's minimum
    /// corner).
    pub pos: Vec<Vec3>,
    /// Particle velocities in m/s.
    pub vel: Vec<Vec3>,
    /// APIC: per particle, the gradient of each velocity component (m/s
    /// per cell).
    pub aff: Vec<[Vec3; 3]>,
    /// Substeps taken so far (seeds the emission jitter).
    pub substeps: u64,
}

/// A cached frame of a liquid domain.
#[derive(Clone, Debug)]
pub struct LiquidFrame {
    pub res: [usize; 3],
    pub min: Vec3,
    pub dx: f32,
    /// The liquid surface in world space (empty when Mesh is off).
    pub mesh: Arc<Mesh>,
    /// Particle positions in world space (Show FLIP Particles).
    pub particles: Option<Arc<Vec<Vec3>>>,
    pub count: usize,
    /// The particles' volume (count × the sampled volume per particle).
    pub particle_volume: f32,
    /// Only on the frame the solver continues from.
    pub state: Option<Box<LiquidState>>,
}

impl LiquidFrame {
    pub fn bytes(&self) -> usize {
        self.mesh.positions.len() * 12 + self.mesh.faces.len() * 48 + self.particles.as_ref().map_or(0, |p| p.len() * 12) + self.state.as_ref().map_or(0, |s| s.pos.len() * 24 + s.aff.len() * 36)
    }

    /// Volume enclosed by the surface mesh.
    pub fn volume(&self) -> f32 {
        mesh_volume(&self.mesh)
    }

    /// The frame for the cache: the full state dropped.
    pub fn display_copy(&self) -> LiquidFrame {
        LiquidFrame { state: None, ..self.clone() }
    }
}

/// Signed volume of a closed triangle mesh.
pub fn mesh_volume(m: &Mesh) -> f32 {
    let mut v = 0.0f64;
    for f in &m.faces {
        for k in 1..f.verts.len().saturating_sub(1) {
            let (a, b, c) = (m.positions[f.verts[0] as usize], m.positions[f.verts[k] as usize], m.positions[f.verts[k + 1] as usize]);
            v += a.dot(b.cross(c)) as f64;
        }
    }
    (v / 6.0) as f32
}

// ---------------------------------------------------------------------------
// Setup

/// The domain's grid: the bounds of its own mesh (before the Fluid
/// modifier replaces it with the liquid).
fn domain_grid(scene: &Scene, obj: usize, s: &FluidSettings) -> Option<Grid> {
    let ObjectData::Mesh(mi) = scene.objects.get(obj)?.data else { return None };
    let m = scene.meshes.get(mi)?;
    if m.positions.is_empty() {
        return None;
    }
    let wm = scene.world_matrix(obj);
    let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for p in &m.positions {
        let w = wm.transform_point(*p);
        lo = lo.min(w);
        hi = hi.max(w);
    }
    let size = hi - lo;
    let longest = size.max_elem().max(1e-4);
    let dx = longest / s.resolution_max.clamp(4, 256) as f32;
    let n = [(size.x / dx).round().max(1.0) as usize, (size.y / dx).round().max(1.0) as usize, (size.z / dx).round().max(1.0) as usize];
    Some(Grid { n, min: lo, dx })
}

fn in_group(scene: &Scene, obj: usize, group: &str) -> bool {
    if group.is_empty() {
        return true;
    }
    let Some(c) = scene.collections.iter().position(|c| c.name == group) else { return true };
    // The object's collection or one of its parents.
    let mut cur = Some(scene.objects[obj].collection);
    let mut guard = 0;
    while let Some(ci) = cur {
        if ci == c {
            return true;
        }
        cur = scene.collections.get(ci).and_then(|x| x.parent);
        guard += 1;
        if guard > scene.collections.len() {
            break;
        }
    }
    false
}

fn band(g: &Grid, dist: f32, plane: bool) -> f32 {
    let b = dist * g.dx;
    if plane {
        b.max(g.dx * 0.5)
    } else {
        b.max(g.dx * 1e-3)
    }
}

/// Effector obstacles rasterised: true for blocked cells.
fn obstacles(scene: &Scene, obj: usize, s: &FluidSettings, g: &Grid) -> Vec<bool> {
    let mut solid = vec![false; g.cells()];
    for (i, o) in scene.objects.iter().enumerate() {
        if i == obj || !in_group(scene, i, &s.effector_group) {
            continue;
        }
        for m in &o.modifiers {
            let Modifier::Physics(PhysicsModifier::Fluid(f)) = m else { continue };
            if f.fluid_type != FluidType::Effector || !f.use_effector || f.effector_type != super::settings::EffectorType::Collision {
                continue;
            }
            let Some(tm) = world_trimesh(scene, i) else { continue };
            let mask = rasterize(g, &tm, band(g, f.effector_surface_distance, f.use_plane_init), 1.0, f.use_plane_init);
            for (sd, &mk) in solid.iter_mut().zip(&mask) {
                if mk > 0.0 {
                    *sd = true;
                }
            }
        }
    }
    solid
}

struct Flow {
    mask: Vec<bool>,
    behavior: FlowBehavior,
    vel: Vec3,
}

/// Liquid flows rasterised. Geometry flows only on the start frame.
fn flows(scene: &Scene, obj: usize, s: &FluidSettings, g: &Grid, first: bool) -> Vec<Flow> {
    let mut out = Vec::new();
    for (i, o) in scene.objects.iter().enumerate() {
        if i == obj || !in_group(scene, i, &s.fluid_group) {
            continue;
        }
        for m in &o.modifiers {
            let Modifier::Physics(PhysicsModifier::Fluid(f)) = m else { continue };
            if f.fluid_type != FluidType::Flow || f.flow_type != FlowType::Liquid || !f.use_flow || f.flow_source != FlowSource::Mesh {
                continue;
            }
            match f.flow_behavior {
                FlowBehavior::Geometry if !first => continue,
                FlowBehavior::Inflow | FlowBehavior::Outflow if !f.use_inflow => continue,
                _ => {}
            }
            let Some(tm) = world_trimesh(scene, i) else { continue };
            let mask = rasterize(g, &tm, band(g, f.surface_distance, f.use_plane_init), 1.0, f.use_plane_init).iter().map(|&x| x > 0.0).collect();
            let vel = if f.use_initial_velocity { Vec3::from_array(f.velocity_coord) } else { Vec3::ZERO };
            out.push(Flow { mask, behavior: f.flow_behavior, vel });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Grid helpers

fn comp(v: Vec3, a: usize) -> f32 {
    v[a]
}

fn with(mut v: Vec3, a: usize, x: f32) -> Vec3 {
    match a {
        0 => v.x = x,
        1 => v.y = x,
        _ => v.z = x,
    }
    v
}

/// Size of the face grid of axis `a`.
fn face_n(n: [usize; 3], a: usize) -> [usize; 3] {
    let mut f = n;
    f[a] += 1;
    f
}

fn unravel(i: usize, n: [usize; 3]) -> [usize; 3] {
    [i % n[0], (i / n[0]) % n[1], i / (n[0] * n[1])]
}

fn ravel(c: [usize; 3], n: [usize; 3]) -> usize {
    c[0] + n[0] * (c[1] + n[1] * c[2])
}

/// The 6 neighbours of item `i` in an `n` grid (u32::MAX outside), in the
/// order -X, +X, -Y, +Y, -Z, +Z.
fn neighbors(i: usize, n: [usize; 3]) -> [u32; 6] {
    let c = unravel(i, n);
    let mut out = [u32::MAX; 6];
    for a in 0..3 {
        if c[a] > 0 {
            let mut d = c;
            d[a] -= 1;
            out[2 * a] = ravel(d, n) as u32;
        }
        if c[a] + 1 < n[a] {
            let mut d = c;
            d[a] += 1;
            out[2 * a + 1] = ravel(d, n) as u32;
        }
    }
    out
}

/// Offset of the samples of face grid `a` (cell units).
fn face_off(a: usize) -> Vec3 {
    with(Vec3::splat(0.5), a, 0.0)
}

/// Velocity at grid position `q` from the three face grids.
fn vel_at(u: &[Vec<f32>; 3], n: [usize; 3], q: Vec3) -> Vec3 {
    Vec3::new(sample(&u[0], face_n(n, 0), q, face_off(0)), sample(&u[1], face_n(n, 1), q, face_off(1)), sample(&u[2], face_n(n, 2), q, face_off(2)))
}

/// Cell of a grid position, clamped into the grid.
fn cell_of(q: Vec3, n: [usize; 3]) -> [usize; 3] {
    [(q.x.floor().max(0.0) as usize).min(n[0] - 1), (q.y.floor().max(0.0) as usize).min(n[1] - 1), (q.z.floor().max(0.0) as usize).min(n[2] - 1)]
}

/// Particles sorted by cell (a stable counting sort).
struct Bins {
    start: Vec<u32>,
    order: Vec<u32>,
}

impl Bins {
    fn new(pos: &[Vec3], n: [usize; 3]) -> Bins {
        let cells = n[0] * n[1] * n[2];
        let mut start = vec![0u32; cells + 1];
        let cell: Vec<usize> = pos.iter().map(|&q| ravel(cell_of(q, n), n)).collect();
        for &c in &cell {
            start[c + 1] += 1;
        }
        for c in 0..cells {
            start[c + 1] += start[c];
        }
        let mut fill = start.clone();
        let mut order = vec![0u32; pos.len()];
        for (p, &c) in cell.iter().enumerate() {
            order[fill[c] as usize] = p as u32;
            fill[c] += 1;
        }
        Bins { start, order }
    }
    fn of(&self, c: usize) -> &[u32] {
        &self.order[self.start[c] as usize..self.start[c + 1] as usize]
    }
    fn count(&self, c: usize) -> usize {
        (self.start[c + 1] - self.start[c]) as usize
    }
}

/// Visit the particles of the cells within `r` (per axis, below/above) of
/// cell `c`, in a fixed order.
fn near(bins: &Bins, n: [usize; 3], lo: [isize; 3], hi: [isize; 3], mut f: impl FnMut(usize)) {
    for z in lo[2].max(0)..=hi[2].min(n[2] as isize - 1) {
        for y in lo[1].max(0)..=hi[1].min(n[1] as isize - 1) {
            for x in lo[0].max(0)..=hi[0].min(n[0] as isize - 1) {
                for &p in bins.of(ravel([x as usize, y as usize, z as usize], n)) {
                    f(p as usize);
                }
            }
        }
    }
}

fn tent(d: f32) -> f32 {
    (1.0 - d.abs()).max(0.0)
}

/// Particle velocities to face grid `a`: (velocity, weight) per face.
fn p2g(st: &LiquidState, bins: &Bins, n: [usize; 3], a: usize, apic: bool) -> Vec<(f32, f32)> {
    let fnn = face_n(n, a);
    let mut out = vec![(0.0f32, 0.0f32); fnn[0] * fnn[1] * fnn[2]];
    par_for(&mut out, |idx, o| {
        let c = unravel(idx, fnn);
        let fp = Vec3::new(c[0] as f32 + 0.5, c[1] as f32 + 0.5, c[2] as f32 + 0.5);
        let fp = with(fp, a, c[a] as f32);
        let mut lo = [c[0] as isize - 1, c[1] as isize - 1, c[2] as isize - 1];
        let mut hi = [c[0] as isize + 1, c[1] as isize + 1, c[2] as isize + 1];
        hi[a] = c[a] as isize;
        lo[a] = c[a] as isize - 1;
        let (mut acc, mut ws) = (0.0f32, 0.0f32);
        near(bins, n, lo, hi, |p| {
            let d = fp - st.pos[p];
            let w = tent(d.x) * tent(d.y) * tent(d.z);
            if w > 0.0 {
                let mut v = comp(st.vel[p], a);
                if apic {
                    v += st.aff[p][a].dot(d);
                }
                acc += w * v;
                ws += w;
            }
        });
        if ws > 0.0 {
            *o = (acc / ws, ws);
        }
    });
    out
}

/// Particle density at cell centres (1 = as densely as sampled).
/// Cells next to the domain's sides see less of the kernel (no particles
/// beyond): each axis's share of an evenly sampled cell's weight.
fn density(st: &LiquidState, bins: &Bins, n: [usize; 3], ppa: usize) -> Vec<f32> {
    let norm: Vec<Vec<f32>> = (0..3)
        .map(|a| {
            (0..n[a])
                .map(|i| {
                    let mut w = 0.0;
                    for j in i.saturating_sub(1)..(i + 2).min(n[a]) {
                        for k in 0..ppa {
                            w += tent(j as f32 + (k as f32 + 0.5) / ppa as f32 - (i as f32 + 0.5));
                        }
                    }
                    w / ppa as f32
                })
                .collect()
        })
        .collect();
    let mut out = vec![0.0f32; n[0] * n[1] * n[2]];
    par_for(&mut out, |idx, o| {
        let c = unravel(idx, n);
        let ppc = norm[0][c[0]] * norm[1][c[1]] * norm[2][c[2]] * (ppa * ppa * ppa) as f32;
        let cp = Vec3::new(c[0] as f32 + 0.5, c[1] as f32 + 0.5, c[2] as f32 + 0.5);
        let lo = [c[0] as isize - 1, c[1] as isize - 1, c[2] as isize - 1];
        let hi = [c[0] as isize + 1, c[1] as isize + 1, c[2] as isize + 1];
        let mut ws = 0.0f32;
        near(bins, n, lo, hi, |p| {
            let d = cp - st.pos[p];
            ws += tent(d.x) * tent(d.y) * tent(d.z);
        });
        *o = ws / ppc;
    });
    out
}

/// A symmetric system `diag[i] x[i] - coef Σ x[nb] = b` over the items
/// with `diag > 0`.
struct System {
    diag: Vec<f32>,
    nb: Vec<[u32; 6]>,
    coef: f32,
}

impl System {
    fn apply(&self, x: &[f32], out: &mut [f32]) {
        par_for(out, |i, o| {
            let d = self.diag[i];
            if d == 0.0 {
                *o = 0.0;
                return;
            }
            let mut s = 0.0;
            for &c in &self.nb[i] {
                if c != u32::MAX {
                    s += x[c as usize];
                }
            }
            *o = d * x[i] - self.coef * s;
        });
    }

    /// Conjugate gradients with a Jacobi preconditioner.
    fn solve(&self, b: &[f32], max_iter: usize, tol: f64) -> Vec<f32> {
        let n = b.len();
        let mut x = vec![0.0f32; n];
        let mut r = b.to_vec();
        let mut z = vec![0.0f32; n];
        let pre = |r: &[f32], z: &mut [f32]| par_for(z, |i, o| *o = if self.diag[i] > 0.0 { r[i] / self.diag[i] } else { 0.0 });
        pre(&r, &mut z);
        let mut p = z.clone();
        let mut rz = dot(&r, &z);
        let r0 = dot(&r, &r).sqrt();
        if r0 < 1e-12 {
            return x;
        }
        let mut ap = vec![0.0f32; n];
        for _ in 0..max_iter {
            self.apply(&p, &mut ap);
            let pap = dot(&p, &ap);
            if pap.abs() < 1e-30 {
                break;
            }
            let alpha = (rz / pap) as f32;
            par_for(&mut x, |i, o| *o += alpha * p[i]);
            par_for(&mut r, |i, o| *o -= alpha * ap[i]);
            if dot(&r, &r).sqrt() < tol * r0 {
                break;
            }
            pre(&r, &mut z);
            let rz2 = dot(&r, &z);
            let beta = (rz2 / rz) as f32;
            rz = rz2;
            par_for(&mut p, |i, o| *o = z[i] + beta * *o);
        }
        x
    }
}

/// Fill the faces that are not `valid` from valid neighbours, `layers`
/// times (each layer reads only the previous one).
fn extrapolate(u: &mut [f32], valid: &mut [bool], n: [usize; 3], layers: usize) {
    for _ in 0..layers {
        let (pu, pv) = (u.to_vec(), valid.to_vec());
        let mut next: Vec<(f32, bool)> = pu.iter().zip(&pv).map(|(&a, &b)| (a, b)).collect();
        par_for(&mut next, |i, o| {
            if pv[i] {
                return;
            }
            let (mut s, mut k) = (0.0f32, 0);
            for c in neighbors(i, n) {
                if c != u32::MAX && pv[c as usize] {
                    s += pu[c as usize];
                    k += 1;
                }
            }
            if k > 0 {
                *o = (s / k as f32, true);
            }
        });
        for (i, (a, b)) in next.into_iter().enumerate() {
            u[i] = a;
            valid[i] = b;
        }
    }
}

/// Deterministic hash to [0, 1).
fn rand01(a: u64, b: u64, c: u64) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ c.wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    h = h.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    h ^= h >> 33;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

// ---------------------------------------------------------------------------
// The solver

struct Solver<'a> {
    s: &'a FluidSettings,
    n: [usize; 3],
    dx: f32,
    solid: Vec<bool>,
    closed: [bool; 6],
    st: LiquidState,
    /// Particles per cell per axis.
    ppa: usize,
}

/// Cell classes for the solves.
const AIR: u8 = 0;
const LIQUID: u8 = 1;
const SOLID: u8 = 2;

impl Solver<'_> {
    fn cells(&self) -> usize {
        self.n[0] * self.n[1] * self.n[2]
    }

    fn ppc(&self) -> usize {
        self.ppa * self.ppa * self.ppa
    }

    fn apic(&self) -> bool {
        self.s.simulation_method == SimulationMethod::Apic
    }

    fn classes(&self, bins: &Bins) -> Vec<u8> {
        (0..self.cells()).map(|c| if self.solid[c] { SOLID } else if bins.count(c) > 0 { LIQUID } else { AIR }).collect()
    }

    /// The class across side `side` (0..6) of cell `c` (`None`: open
    /// border, which acts as air).
    fn across(&self, cls: &[u8], c: usize, side: usize) -> (u8, Option<usize>) {
        let nb = neighbors(c, self.n)[side];
        if nb == u32::MAX {
            return (if self.closed[side] { SOLID } else { AIR }, None);
        }
        (cls[nb as usize], Some(nb as usize))
    }

    /// Whether the face of axis `a` at face index `f` is a wall, and the
    /// cells on its two sides.
    fn face_cells(&self, a: usize, f: usize) -> (Option<usize>, Option<usize>) {
        let fnn = face_n(self.n, a);
        let c = unravel(f, fnn);
        let lo = (c[a] > 0).then(|| {
            let mut d = c;
            d[a] -= 1;
            ravel(d, self.n)
        });
        let hi = (c[a] < self.n[a]).then(|| ravel(c, self.n));
        (lo, hi)
    }

    fn face_class(&self, cls: &[u8], a: usize, f: usize) -> (u8, u8) {
        let (lo, hi) = self.face_cells(a, f);
        let side = |c: Option<usize>, s: usize| match c {
            Some(c) => cls[c],
            None => {
                if self.closed[s] {
                    SOLID
                } else {
                    AIR
                }
            }
        };
        (side(lo, 2 * a), side(hi, 2 * a + 1))
    }

    /// Seed `count` particles in cell `c` (jittered sub-cell lattice).
    fn seed(&mut self, c: usize, vel: Vec3, salt: u64) {
        let cc = unravel(c, self.n);
        let k = self.ppa;
        let r = self.s.particle_randomness.max(0.0);
        for m in 0..self.ppc() {
            let sub = [m % k, (m / k) % k, m / (k * k)];
            let mut q = Vec3::ZERO;
            for a in 0..3 {
                let j = (rand01(c as u64, self.st.substeps ^ salt, (m * 3 + a) as u64) - 0.5) * r;
                let x = cc[a] as f32 + ((sub[a] as f32 + 0.5 + j) / k as f32).clamp(0.001, 0.999);
                q = with(q, a, x);
            }
            self.st.pos.push(q);
            self.st.vel.push(vel);
            if self.apic() {
                self.st.aff.push([Vec3::ZERO; 3]);
            }
        }
    }

    fn retain(&mut self, keep: &[bool]) {
        let mut k = 0;
        let apic = !self.st.aff.is_empty();
        for i in 0..keep.len() {
            if keep[i] {
                self.st.pos[k] = self.st.pos[i];
                self.st.vel[k] = self.st.vel[i];
                if apic {
                    self.st.aff[k] = self.st.aff[i];
                }
                k += 1;
            }
        }
        self.st.pos.truncate(k);
        self.st.vel.truncate(k);
        if apic {
            self.st.aff.truncate(k);
        }
    }

    fn emit(&mut self, flows: &[Flow]) {
        if flows.is_empty() {
            return;
        }
        // Outflow first.
        let outs: Vec<&Flow> = flows.iter().filter(|f| f.behavior == FlowBehavior::Outflow).collect();
        if !outs.is_empty() {
            let n = self.n;
            let keep: Vec<bool> = self.st.pos.iter().map(|&q| !outs.iter().any(|f| f.mask[ravel(cell_of(q, n), n)])).collect();
            self.retain(&keep);
        }
        let max = if self.s.sys_particle_maximum > 0 { self.s.sys_particle_maximum as usize } else { usize::MAX };
        let mut filled = vec![false; self.cells()];
        for &q in &self.st.pos {
            filled[ravel(cell_of(q, self.n), self.n)] = true;
        }
        for (fi, f) in flows.iter().enumerate() {
            if f.behavior == FlowBehavior::Outflow {
                continue;
            }
            for c in 0..self.cells() {
                if !f.mask[c] || self.solid[c] || filled[c] {
                    continue;
                }
                if self.st.pos.len() + self.ppc() > max {
                    return;
                }
                self.seed(c, f.vel, fi as u64 + 1);
                filled[c] = true;
            }
        }
    }

    /// One substep.
    fn substep(&mut self, dt: f32, flows: &[Flow], gravity: Vec3, fields: &super::field::Effectors, time: f32) {
        self.emit(flows);
        self.st.substeps += 1;
        if self.st.pos.is_empty() {
            return;
        }
        let n = self.n;
        let apic = self.apic();
        if apic && self.st.aff.len() != self.st.pos.len() {
            self.st.aff = vec![[Vec3::ZERO; 3]; self.st.pos.len()];
        }
        let bins = Bins::new(&self.st.pos, n);
        let cls = self.classes(&bins);
        // Transfer to the grid.
        let mut u: [Vec<f32>; 3] = Default::default();
        let mut valid: [Vec<bool>; 3] = Default::default();
        for a in 0..3 {
            let t = p2g(&self.st, &bins, n, a, apic);
            u[a] = t.iter().map(|x| x.0).collect();
            valid[a] = t.iter().map(|x| x.1 > 0.0).collect();
            extrapolate(&mut u[a], &mut valid[a], face_n(n, a), 2);
        }
        let old = u.clone();
        // Forces.
        for a in 0..3 {
            let g = comp(gravity, a) * dt;
            if g != 0.0 {
                par_for(&mut u[a], |_, x| *x += g);
            }
        }
        if !fields.is_empty() {
            for c in 0..self.cells() {
                if cls[c] != LIQUID {
                    continue;
                }
                let cc = unravel(c, n);
                let q = Vec3::new(cc[0] as f32 + 0.5, cc[1] as f32 + 0.5, cc[2] as f32 + 0.5);
                let f = fields.force(q, vel_at(&u, n, q), time) * (0.5 * dt);
                for a in 0..3 {
                    let fnn = face_n(n, a);
                    let lo = ravel(cc, fnn);
                    let mut hc = cc;
                    hc[a] += 1;
                    let hi = ravel(hc, fnn);
                    u[a][lo] += comp(f, a);
                    u[a][hi] += comp(f, a);
                }
            }
        }
        // Viscosity.
        let nu = self.s.kinematic_viscosity();
        if nu > 0.0 {
            let alpha = nu * dt / (self.dx * self.dx);
            for a in 0..3 {
                self.diffuse(&mut u[a], &cls, a, alpha);
            }
        }
        // Pressure.
        self.project(&mut u, &cls, dt);
        for a in 0..3 {
            let fnn = face_n(n, a);
            let mut ok: Vec<bool> = (0..u[a].len())
                .map(|f| {
                    let (l, h) = self.face_class(&cls, a, f);
                    l == LIQUID || h == LIQUID || l == SOLID || h == SOLID
                })
                .collect();
            extrapolate(&mut u[a], &mut ok, fnn, 4);
        }
        // Back to the particles.
        let ratio = self.s.flip_ratio.clamp(0.0, 1.0);
        let st = &mut self.st;
        let pos = &st.pos;
        if apic {
            let mut pv: Vec<(Vec3, [Vec3; 3])> = vec![(Vec3::ZERO, [Vec3::ZERO; 3]); pos.len()];
            par_for(&mut pv, |p, o| *o = apic_g2p(&u, n, pos[p]));
            for (p, (v, c)) in pv.into_iter().enumerate() {
                st.vel[p] = v;
                st.aff[p] = c;
            }
        } else {
            let vel = &mut st.vel;
            par_for(vel, |p, v| {
                let q = pos[p];
                let new = vel_at(&u, n, q);
                let dv = new - vel_at(&old, n, q);
                *v = (*v + dv) * ratio + new * (1.0 - ratio);
            });
        }
        // Move.
        self.advect(&u, dt);
        self.correct_volume();
        self.tidy();
    }

    /// Implicit viscosity on face grid `a`.
    fn diffuse(&self, u: &mut [f32], cls: &[u8], a: usize, alpha: f32) {
        let fnn = face_n(self.n, a);
        let nf = u.len();
        let unknown: Vec<bool> = (0..nf)
            .map(|f| {
                let (l, h) = self.face_class(cls, a, f);
                (l == LIQUID || h == LIQUID) && l != SOLID && h != SOLID
            })
            .collect();
        if !unknown.iter().any(|&x| x) {
            return;
        }
        let mut diag = vec![0.0f32; nf];
        let mut nb = vec![[u32::MAX; 6]; nf];
        let mut b = vec![0.0f32; nf];
        for f in 0..nf {
            if !unknown[f] {
                continue;
            }
            let mut d = 1.0;
            let mut rhs = u[f];
            for (k, c) in neighbors(f, fnn).into_iter().enumerate() {
                if c == u32::MAX {
                    continue;
                }
                d += alpha;
                if unknown[c as usize] {
                    nb[f][k] = c;
                } else {
                    rhs += alpha * u[c as usize];
                }
            }
            diag[f] = d;
            b[f] = rhs;
        }
        let x = System { diag, nb, coef: alpha }.solve(&b, 200, 1e-5);
        for f in 0..nf {
            if unknown[f] {
                u[f] = x[f];
            }
        }
    }

    /// Curvature (1/m) at cell centres from the particle density.
    fn curvature(&self, rho: &[f32]) -> Vec<f32> {
        let n = self.n;
        let phi: Vec<f32> = rho.iter().map(|&r| 0.5 - r.clamp(0.0, 1.0)).collect();
        let at = |c: [isize; 3]| -> f32 {
            let cc = [c[0].clamp(0, n[0] as isize - 1) as usize, c[1].clamp(0, n[1] as isize - 1) as usize, c[2].clamp(0, n[2] as isize - 1) as usize];
            phi[ravel(cc, n)]
        };
        let normal = |c: [isize; 3]| -> Vec3 {
            let g = Vec3::new(
                at([c[0] + 1, c[1], c[2]]) - at([c[0] - 1, c[1], c[2]]),
                at([c[0], c[1] + 1, c[2]]) - at([c[0], c[1] - 1, c[2]]),
                at([c[0], c[1], c[2] + 1]) - at([c[0], c[1], c[2] - 1]),
            );
            let l = g.length();
            if l > 1e-6 {
                g / l
            } else {
                Vec3::ZERO
            }
        };
        let mut out = vec![0.0f32; self.cells()];
        let dx = self.dx;
        par_for(&mut out, |i, o| {
            let c = unravel(i, n);
            let c = [c[0] as isize, c[1] as isize, c[2] as isize];
            let k = (normal([c[0] + 1, c[1], c[2]]).x - normal([c[0] - 1, c[1], c[2]]).x + normal([c[0], c[1] + 1, c[2]]).y - normal([c[0], c[1] - 1, c[2]]).y + normal([c[0], c[1], c[2] + 1]).z
                - normal([c[0], c[1], c[2] - 1]).z)
                / (2.0 * dx);
            *o = k.clamp(-1.0 / dx, 1.0 / dx);
        });
        out
    }

    /// Make `u` divergence-free in the liquid cells.
    fn project(&self, u: &mut [Vec<f32>; 3], cls: &[u8], dt: f32) {
        let n = self.n;
        let cells = self.cells();
        // Walls.
        for a in 0..3 {
            for f in 0..u[a].len() {
                let (l, h) = self.face_class(cls, a, f);
                if l == SOLID || h == SOLID {
                    u[a][f] = 0.0;
                }
            }
        }
        // Surface tension: the pressure jump σκ in the air next to liquid.
        let ghost: Vec<f32> = if self.s.surface_tension > 0.0 {
            let bins = Bins::new(&self.st.pos, n);
            let rho = density(&self.st, &bins, n, self.ppa);
            let kappa = self.curvature(&rho);
            (0..cells)
                .map(|c| {
                    if cls[c] != AIR {
                        return 0.0;
                    }
                    let mut s = 0.0;
                    let mut k = 0;
                    for nb in neighbors(c, n) {
                        if nb != u32::MAX && cls[nb as usize] == LIQUID {
                            s += 0.5 * (kappa[c] + kappa[nb as usize]);
                            k += 1;
                        }
                    }
                    if k > 0 {
                        dt * self.s.surface_tension * s / k as f32
                    } else {
                        0.0
                    }
                })
                .collect()
        } else {
            vec![0.0; cells]
        };
        let mut diag = vec![0.0f32; cells];
        let mut nb = vec![[u32::MAX; 6]; cells];
        let mut b = vec![0.0f32; cells];
        let uidx = |a: usize, c: [usize; 3]| ravel(c, face_n(n, a));
        for c in 0..cells {
            if cls[c] != LIQUID {
                continue;
            }
            let cc = unravel(c, n);
            let mut div = 0.0;
            for a in 0..3 {
                let mut hc = cc;
                hc[a] += 1;
                div += u[a][uidx(a, hc)] - u[a][uidx(a, cc)];
            }
            let mut rhs = -div * self.dx;
            let mut d = 0.0;
            for side in 0..6 {
                let (k, other) = self.across(cls, c, side);
                match k {
                    LIQUID => {
                        d += 1.0;
                        nb[c][side] = other.unwrap() as u32;
                    }
                    AIR => {
                        d += 1.0;
                        if let Some(o) = other {
                            rhs += ghost[o];
                        }
                    }
                    _ => {}
                }
            }
            diag[c] = d;
            b[c] = rhs;
        }
        let x = System { diag, nb, coef: 1.0 }.solve(&b, 600, 1e-5);
        let value = |c: Option<usize>| -> Option<f32> {
            match c {
                Some(c) => match cls[c] {
                    LIQUID => Some(x[c]),
                    AIR => Some(ghost[c]),
                    _ => None,
                },
                None => Some(0.0),
            }
        };
        for a in 0..3 {
            let fnn = face_n(n, a);
            let snap: Vec<(Option<usize>, Option<usize>)> = (0..u[a].len()).map(|f| self.face_cells(a, f)).collect();
            par_for(&mut u[a], |f, uf| {
                let (lo, hi) = snap[f];
                let (cl, ch) = self.face_class(cls, a, f);
                if cl == SOLID || ch == SOLID {
                    *uf = 0.0;
                    return;
                }
                if cl != LIQUID && ch != LIQUID {
                    return;
                }
                if let (Some(pl), Some(ph)) = (value(lo), value(hi)) {
                    *uf -= (ph - pl) / self.dx;
                }
                let _ = fnn;
            });
        }
    }

    /// Move the particles through `u`.
    fn advect(&mut self, u: &[Vec<f32>; 3], dt: f32) {
        let n = self.n;
        let dx = self.dx;
        let solid = &self.solid;
        let closed = self.closed;
        let pos = std::mem::take(&mut self.st.pos);
        let mut moved: Vec<(Vec3, Vec3, bool)> = pos.iter().zip(&self.st.vel).map(|(&q, &v)| (q, v, true)).collect();
        par_for(&mut moved, |_, (q, v, alive)| {
            let v1 = vel_at(u, n, *q);
            let travel = (v1 * (dt / dx)).abs().max_elem();
            let k = (travel.ceil() as usize).clamp(1, 16);
            let h = dt / k as f32;
            for _ in 0..k {
                let a = vel_at(u, n, *q);
                let mid = *q + a * (0.5 * h / dx);
                let b = vel_at(u, n, mid);
                let target = *q + b * (h / dx);
                match resolve(*q, target, n, solid, closed) {
                    Some((p, hit)) => {
                        *q = p;
                        for a in 0..3 {
                            if hit[a] {
                                *v = with(*v, a, 0.0);
                            }
                        }
                    }
                    None => {
                        *alive = false;
                        return;
                    }
                }
            }
        });
        self.st.pos = moved.iter().map(|m| m.0).collect();
        self.st.vel = moved.iter().map(|m| m.1).collect();
        let keep: Vec<bool> = moved.iter().map(|m| m.2).collect();
        if keep.iter().any(|&k| !k) {
            self.retain(&keep);
        }
    }

    /// Particles inside obstacles.
    fn tidy(&mut self) {
        let n = self.n;
        // Particles inside obstacles (the obstacle moved onto them).
        let mut keep = vec![true; self.st.pos.len()];
        for p in 0..self.st.pos.len() {
            let c = ravel(cell_of(self.st.pos[p], n), n);
            if !self.solid[c] {
                continue;
            }
            if self.s.delete_in_obstacle {
                keep[p] = false;
                continue;
            }
            // Out to a free neighbour cell, else delete.
            let cc = cell_of(self.st.pos[p], n);
            let mut done = false;
            for (side, nb) in neighbors(c, n).into_iter().enumerate() {
                if nb != u32::MAX && !self.solid[nb as usize] {
                    let a = side / 2;
                    let x = if side % 2 == 0 { cc[a] as f32 - 0.05 } else { cc[a] as f32 + 1.05 };
                    self.st.pos[p] = with(self.st.pos[p], a, x);
                    self.st.vel[p] = with(self.st.vel[p], a, 0.0);
                    done = true;
                    break;
                }
            }
            keep[p] = done;
        }
        if keep.iter().any(|&k| !k) {
            self.retain(&keep);
        }
    }

    /// Even out the particle density (positions only): over-packed cells
    /// push particles out, thinned-out interior cells pull them in.
    fn correct_volume(&mut self) {
        if self.st.pos.is_empty() {
            return;
        }
        let n = self.n;
        let cells = self.cells();
        let bins = Bins::new(&self.st.pos, n);
        let cls = self.classes(&bins);
        let rho = density(&self.st, &bins, n, self.ppa);
        let mut diag = vec![0.0f32; cells];
        let mut nb = vec![[u32::MAX; 6]; cells];
        let mut b = vec![0.0f32; cells];
        let mut any = false;
        for c in 0..cells {
            if cls[c] != LIQUID {
                continue;
            }
            let mut d = 0.0;
            let mut surface = false;
            for side in 0..6 {
                let (k, other) = self.across(&cls, c, side);
                match k {
                    LIQUID => {
                        d += 1.0;
                        nb[c][side] = other.unwrap() as u32;
                    }
                    AIR => {
                        d += 1.0;
                        surface = true;
                    }
                    _ => {}
                }
            }
            diag[c] = d;
            let mut excess = (rho[c] - 1.02).max(0.0);
            // Interior cells thinned out pull particles back in.
            if !surface && rho[c] < 0.9 {
                excess = rho[c] - 0.98;
            }
            if excess != 0.0 {
                any = true;
            }
            b[c] = 0.5 * excess;
        }
        if !any {
            return;
        }
        let phi = System { diag, nb, coef: 1.0 }.solve(&b, 200, 1e-4);
        // Displacement on faces (cells), -grad phi.
        let mut d: [Vec<f32>; 3] = Default::default();
        for a in 0..3 {
            let fnn = face_n(n, a);
            let nf = fnn[0] * fnn[1] * fnn[2];
            let mut v = vec![0.0f32; nf];
            let mut ok = vec![false; nf];
            for f in 0..nf {
                let (lo, hi) = self.face_cells(a, f);
                let (cl, ch) = self.face_class(&cls, a, f);
                if cl == SOLID || ch == SOLID {
                    ok[f] = true;
                    continue;
                }
                if cl != LIQUID && ch != LIQUID {
                    continue;
                }
                let val = |c: Option<usize>| c.map_or(0.0, |c| if cls[c] == LIQUID { phi[c] } else { 0.0 });
                v[f] = -(val(hi) - val(lo));
                ok[f] = true;
            }
            extrapolate(&mut v, &mut ok, fnn, 1);
            d[a] = v;
        }
        let solid = &self.solid;
        let closed = self.closed;
        let mut moved: Vec<(Vec3, bool)> = self.st.pos.iter().map(|&q| (q, true)).collect();
        par_for(&mut moved, |_, (q, alive)| {
            let mut s = vel_at(&d, n, *q);
            let l = s.length();
            if l > 0.5 {
                s = s * (0.5 / l);
            }
            match resolve(*q, *q + s, n, solid, closed) {
                Some((p, _)) => *q = p,
                None => *alive = false,
            }
        });
        self.st.pos = moved.iter().map(|m| m.0).collect();
        let keep: Vec<bool> = moved.iter().map(|m| m.1).collect();
        if keep.iter().any(|&k| !k) {
            self.retain(&keep);
        }
    }
}

/// APIC grid-to-particle: velocity and per-axis gradients (per cell).
fn apic_g2p(u: &[Vec<f32>; 3], n: [usize; 3], q: Vec3) -> (Vec3, [Vec3; 3]) {
    let mut v = Vec3::ZERO;
    let mut c = [Vec3::ZERO; 3];
    for a in 0..3 {
        let fnn = face_n(n, a);
        let off = face_off(a);
        let p = q - off;
        let base = [p.x.floor() as isize, p.y.floor() as isize, p.z.floor() as isize];
        let fr = Vec3::new(p.x - base[0] as f32, p.y - base[1] as f32, p.z - base[2] as f32);
        let (mut val, mut grad) = (0.0f32, Vec3::ZERO);
        for dz in 0..2 {
            for dy in 0..2 {
                for dxx in 0..2 {
                    let idx = [(base[0] + dxx).clamp(0, fnn[0] as isize - 1) as usize, (base[1] + dy).clamp(0, fnn[1] as isize - 1) as usize, (base[2] + dz).clamp(0, fnn[2] as isize - 1) as usize];
                    let x = u[a][ravel(idx, fnn)];
                    let wx = if dxx == 1 { fr.x } else { 1.0 - fr.x };
                    let wy = if dy == 1 { fr.y } else { 1.0 - fr.y };
                    let wz = if dz == 1 { fr.z } else { 1.0 - fr.z };
                    let gx = if dxx == 1 { 1.0 } else { -1.0 };
                    let gy = if dy == 1 { 1.0 } else { -1.0 };
                    let gz = if dz == 1 { 1.0 } else { -1.0 };
                    val += wx * wy * wz * x;
                    grad += Vec3::new(gx * wy * wz, wx * gy * wz, wx * wy * gz) * x;
                }
            }
        }
        v = with(v, a, val);
        c[a] = grad;
    }
    (v, c)
}

/// Where a particle moving from `from` to `to` (grid units) ends up:
/// stopped by obstacles and closed sides (and which axes were stopped), or
/// `None` when it leaves through an open side.
fn resolve(from: Vec3, to: Vec3, n: [usize; 3], solid: &[bool], closed: [bool; 6]) -> Option<(Vec3, [bool; 3])> {
    const EPS: f32 = 1e-3;
    let mut hit = [false; 3];
    let mut t = to;
    for a in 0..3 {
        let x = comp(t, a);
        if x < 0.0 {
            if !closed[2 * a] {
                return None;
            }
            // Stay where it was across the wall (clamping onto the wall
            // would stack particles there).
            t = with(t, a, comp(from, a).clamp(EPS, n[a] as f32 - EPS));
            hit[a] = true;
        } else if x >= n[a] as f32 {
            if !closed[2 * a + 1] {
                return None;
            }
            t = with(t, a, comp(from, a).clamp(EPS, n[a] as f32 - EPS));
            hit[a] = true;
        }
    }
    let blocked = |p: Vec3| solid[ravel(cell_of(p, n), n)];
    if !blocked(t) {
        return Some((t, hit));
    }
    // Slide: take each axis's move only when it stays free.
    let mut c = from;
    for a in 0..3 {
        let trial = with(c, a, comp(t, a));
        if !blocked(trial) {
            c = trial;
        } else {
            hit[a] = true;
        }
    }
    Some((c, hit))
}

// ---------------------------------------------------------------------------
// Stepping

fn gravity_of(scene: &Scene, s: &FluidSettings) -> Vec3 {
    let gs = &scene.physics.gravity;
    let g = if gs.use_gravity { Vec3::from_array(gs.gravity) } else { Vec3::from_array(s.gravity) };
    g * s.effector_weights.gravity
}

fn make_solver<'a>(scene: &Scene, obj: usize, s: &'a FluidSettings, g: &Grid, st: LiquidState) -> Solver<'a> {
    Solver { s, n: g.n, dx: g.dx, solid: obstacles(scene, obj, s, g), closed: s.closed_borders(), st, ppa: s.particle_number.clamp(1, 5) as usize }
}

fn finish(s: &FluidSettings, g: &Grid, st: LiquidState) -> LiquidFrame {
    let ppa = s.particle_number.clamp(1, 5) as f32;
    let pv = (g.dx / ppa).powi(3);
    let mesh = if s.use_mesh { surface(&st.pos, g, s) } else { Mesh::new() };
    let particles = s.use_flip_particles.then(|| Arc::new(st.pos.iter().map(|&q| g.min + q * g.dx).collect::<Vec<_>>()));
    LiquidFrame { res: g.n, min: g.min, dx: g.dx, mesh: Arc::new(mesh), particles, count: st.pos.len(), particle_volume: st.pos.len() as f32 * pv, state: Some(Box::new(st)) }
}

/// The domain at the start frame: Geometry flows filled in, Inflows
/// seeded.
pub fn initial(scene: &Scene, obj: usize, s: &FluidSettings) -> Option<LiquidFrame> {
    let g = domain_grid(scene, obj, s)?;
    let mut solver = make_solver(scene, obj, s, &g, LiquidState::default());
    let fl = flows(scene, obj, s, &g, true);
    solver.emit(&fl);
    solver.tidy();
    Some(finish(s, &g, solver.st))
}

/// Advance a domain one frame.
pub fn step(scene: &Scene, obj: usize, s: &FluidSettings, prev: &LiquidFrame) -> Option<LiquidFrame> {
    let st = prev.state.as_ref()?.as_ref().clone();
    let g = Grid { n: prev.res, min: prev.min, dx: prev.dx };
    let mut solver = make_solver(scene, obj, s, &g, st);
    let fl = flows(scene, obj, s, &g, false);
    let gravity = gravity_of(scene, s);
    let fields = super::field::Effectors::collect(scene, Some(obj), &s.effector_weights);
    let dt_frame = s.time_scale / scene.fps.max(1e-3);
    let steps = if s.use_adaptive_timesteps {
        let vmax = solver.st.vel.iter().fold(0.0f32, |m, v| m.max(v.abs().max_elem())) + gravity.length() * dt_frame;
        let need = (vmax * dt_frame / (s.cfl_condition.max(0.1) * g.dx)).ceil() as i32;
        need.clamp(s.timesteps_min.max(1), s.timesteps_max.max(1)) as usize
    } else {
        s.timesteps_min.max(1) as usize
    };
    let dt = dt_frame / steps as f32;
    for _ in 0..steps {
        solver.substep(dt, &fl, gravity, &fields, scene.frame_current as f32);
    }
    Some(finish(s, &g, solver.st))
}

/// The surface mesh in the domain object's space, as the Fluid modifier
/// shows it at the scene's frame (`None`: show the domain's own mesh).
pub fn modifier_mesh(scene: &Scene, obj: usize, key: &str, s: &FluidSettings) -> Option<Mesh> {
    if !s.use_mesh || scene.frame_current < s.cache_frame_start {
        return None;
    }
    let f = scene.physics.caches.frame_at(key, scene.frame_current.min(s.cache_frame_end))?;
    let super::cache::SimFrame::Liquid(lf) = &*f else { return None };
    let mut m = (*lf.mesh).clone();
    m.transform(&scene.world_matrix(obj).inverse());
    Some(m)
}

// ---------------------------------------------------------------------------
// Surface

/// The liquid surface (world space) from particle positions in grid units.
pub(super) fn surface(pos: &[Vec3], g: &Grid, s: &FluidSettings) -> Mesh {
    if pos.is_empty() {
        return Mesh::new();
    }
    let preview = s.mesh_generator == MeshGenerator::Union;
    let ms = if preview { 1 } else { s.mesh_scale.clamp(1, 4) as usize };
    let ppa = s.particle_number.clamp(1, 5) as f32;
    // Nodes at (t - 1) / ms cells: one layer outside the grid each side.
    let nn = [g.n[0] * ms + 3, g.n[1] * ms + 3, g.n[2] * ms + 3];
    let spacing = ms as f32 / ppa; // particle spacing in nodes
    let rho = (s.mesh_particle_radius.max(0.5).round() * spacing * s.particle_radius.max(0.1)).clamp(1.0, 7.0);
    let vol = spacing * spacing * spacing;
    let mut f = vec![0.0f32; nn[0] * nn[1] * nn[2]];
    let r = rho.ceil() as isize;
    // Closed sides mirror the liquid, so it fills up to the wall.
    let closed = s.closed_borders();
    let hi_wall = [(g.n[0] * ms + 1) as f32, (g.n[1] * ms + 1) as f32, (g.n[2] * ms + 1) as f32];
    let mut copies = Vec::with_capacity(8);
    for &q in pos {
        copies.clear();
        copies.push(q * ms as f32 + Vec3::splat(1.0));
        for a in 0..3 {
            for c in 0..copies.len() {
                let p = copies[c];
                let x = comp(p, a);
                if closed[2 * a] && x - 1.0 < rho {
                    copies.push(with(p, a, 2.0 - x));
                } else if closed[2 * a + 1] && hi_wall[a] - x < rho {
                    copies.push(with(p, a, 2.0 * hi_wall[a] - x));
                }
            }
        }
        for &t in &copies {
        let base = [t.x.floor() as isize, t.y.floor() as isize, t.z.floor() as isize];
        let w1 = |d: f32| (1.0 - d.abs() / rho).max(0.0) / rho;
        let mut wx = [0.0f32; 16];
        let mut wy = [0.0f32; 16];
        let mut wz = [0.0f32; 16];
        let span = (2 * r + 2) as usize;
        for k in 0..span.min(16) {
            let o = k as isize - r;
            wx[k] = w1(t.x - (base[0] + o) as f32);
            wy[k] = w1(t.y - (base[1] + o) as f32);
            wz[k] = w1(t.z - (base[2] + o) as f32);
        }
        for kz in 0..span.min(16) {
            let z = base[2] + kz as isize - r;
            if z < 0 || z >= nn[2] as isize || wz[kz] == 0.0 {
                continue;
            }
            for ky in 0..span.min(16) {
                let y = base[1] + ky as isize - r;
                if y < 0 || y >= nn[1] as isize || wy[ky] == 0.0 {
                    continue;
                }
                let wyz = wy[ky] * wz[kz] * vol;
                let row = nn[0] * (y as usize + nn[1] * z as usize);
                for kx in 0..span.min(16) {
                    let x = base[0] + kx as isize - r;
                    if x < 0 || x >= nn[0] as isize || wx[kx] == 0.0 {
                        continue;
                    }
                    f[row + x as usize] += wx[kx] * wyz;
                }
            }
        }
        }
    }
    if !preview {
        smooth(&mut f, nn, s.mesh_smoothen_pos.max(0) as usize, true);
        smooth(&mut f, nn, s.mesh_smoothen_neg.max(0) as usize, false);
    }
    // The outermost layer is outside: the surface closes.
    for i in 0..f.len() {
        let c = unravel(i, nn);
        if (0..3).any(|a| c[a] == 0 || c[a] == nn[a] - 1) {
            f[i] = 0.0;
        }
    }
    let phi: Vec<f32> = f.iter().map(|&x| 0.5 - x).collect();
    let h = g.dx / ms as f32;
    let origin = g.min - Vec3::splat(h);
    let mut m = marching_cubes(&phi, nn, origin, h);
    // The liquid ends at the domain's sides.
    let top = g.min + Vec3::new(g.n[0] as f32, g.n[1] as f32, g.n[2] as f32) * g.dx;
    for p in &mut m.positions {
        *p = p.max(g.min).min(top);
    }
    for face in &mut m.faces {
        face.smooth = true;
    }
    m
}

/// Relax the field towards its neighbours' mean, only upwards (`up`) or
/// only downwards.
fn smooth(f: &mut [f32], n: [usize; 3], iters: usize, up: bool) {
    for _ in 0..iters {
        let prev = f.to_vec();
        par_for(f, |i, o| {
            let mut s = 0.0;
            let mut k = 0;
            for c in neighbors(i, n) {
                if c != u32::MAX {
                    s += prev[c as usize];
                    k += 1;
                }
            }
            if k == 0 {
                return;
            }
            let avg = s / k as f32;
            let v = prev[i] + 0.5 * (avg - prev[i]);
            *o = if up { prev[i].max(v) } else { prev[i].min(v) };
        });
    }
}

// ---------------------------------------------------------------------------
// Marching cubes

/// Corner `c` of a cube: (x, y, z) bits.
fn corner(c: usize) -> [usize; 3] {
    [c & 1, (c >> 1) & 1, (c >> 2) & 1]
}

/// The 12 cube edges as (axis, lower corner).
fn edges() -> [(usize, usize); 12] {
    let mut out = [(0, 0); 12];
    let mut k = 0;
    for a in 0..3 {
        for c in 0..8 {
            if (c >> a) & 1 == 0 {
                out[k] = (a, c);
                k += 1;
            }
        }
    }
    out
}

fn edge_id(a: usize, b: usize) -> usize {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let axis = (lo ^ hi).trailing_zeros() as usize;
    edges().iter().position(|&e| e == (axis, lo)).unwrap()
}

/// Per case (bit c set: corner c inside), the surface loops as edge ids,
/// counter-clockwise seen from outside. Built by walking the cube's faces:
/// on each face the crossing points are joined around the inside corners
/// (both, on a face with two opposite inside corners), so neighbouring
/// cubes agree and the surface is closed.
fn table() -> &'static Vec<Vec<Vec<u8>>> {
    static T: OnceLock<Vec<Vec<Vec<u8>>>> = OnceLock::new();
    T.get_or_init(|| {
        let pos = |c: usize| {
            let k = corner(c);
            Vec3::new(k[0] as f32, k[1] as f32, k[2] as f32)
        };
        let mid = |e: usize| {
            let (a, c) = edges()[e];
            pos(c) + with(Vec3::ZERO, a, 0.5)
        };
        let mut faces = Vec::new();
        for a in 0..3 {
            let (b, c) = ((a + 1) % 3, (a + 2) % 3);
            for side in 0..2 {
                let mk = |pb: usize, pc: usize| (side << a) | (pb << b) | (pc << c);
                let cyc = [mk(0, 0), mk(1, 0), mk(1, 1), mk(0, 1)];
                let normal = with(Vec3::ZERO, a, if side == 0 { -1.0 } else { 1.0 });
                faces.push((cyc, normal));
            }
        }
        (0..256usize)
            .map(|case| {
                let inside = |c: usize| (case >> c) & 1 == 1;
                let mut segs: Vec<(usize, usize)> = Vec::new();
                for (cyc, nf) in &faces {
                    let crossings: Vec<usize> = (0..4).filter(|&k| inside(cyc[k]) != inside(cyc[(k + 1) % 4])).collect();
                    let orient = |p: usize, q: usize, inner: Vec3| -> (usize, usize) {
                        let (pp, qq) = (mid(p), mid(q));
                        if (qq - pp).cross(*nf).dot(inner - pp) > 0.0 {
                            (p, q)
                        } else {
                            (q, p)
                        }
                    };
                    match crossings.len() {
                        2 => {
                            let e1 = edge_id(cyc[crossings[0]], cyc[(crossings[0] + 1) % 4]);
                            let e2 = edge_id(cyc[crossings[1]], cyc[(crossings[1] + 1) % 4]);
                            let ins: Vec<Vec3> = cyc.iter().filter(|&&c| inside(c)).map(|&c| pos(c)).collect();
                            let inner = ins.iter().fold(Vec3::ZERO, |s, &p| s + p) / ins.len() as f32;
                            segs.push(orient(e1, e2, inner));
                        }
                        4 => {
                            for k in 0..4 {
                                if inside(cyc[k]) {
                                    let e1 = edge_id(cyc[(k + 3) % 4], cyc[k]);
                                    let e2 = edge_id(cyc[k], cyc[(k + 1) % 4]);
                                    segs.push(orient(e1, e2, pos(cyc[k])));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let mut loops = Vec::new();
                let mut used = vec![false; segs.len()];
                for s0 in 0..segs.len() {
                    if used[s0] {
                        continue;
                    }
                    used[s0] = true;
                    let mut lp = vec![segs[s0].0 as u8];
                    let mut cur = segs[s0].1;
                    while cur != segs[s0].0 {
                        lp.push(cur as u8);
                        let Some(nx) = (0..segs.len()).find(|&k| !used[k] && segs[k].0 == cur) else { break };
                        used[nx] = true;
                        cur = segs[nx].1;
                    }
                    loops.push(lp);
                }
                loops
            })
            .collect()
    })
}

/// Triangulate the surface `phi = 0` (inside negative) of a node grid of
/// size `n` with spacing `h` from `origin`; triangles face outwards.
pub(crate) fn marching_cubes(phi: &[f32], n: [usize; 3], origin: Vec3, h: f32) -> Mesh {
    let tab = table();
    let ed = edges();
    let inside = |v: f32| v < 0.0;
    let nodes = n[0] * n[1] * n[2];
    let mut vid: [Vec<u32>; 3] = [vec![u32::MAX; nodes], vec![u32::MAX; nodes], vec![u32::MAX; nodes]];
    let mut positions = Vec::new();
    let mut faces = Vec::new();
    if n.iter().any(|&x| x < 2) {
        return Mesh::new();
    }
    for z in 0..n[2] - 1 {
        for y in 0..n[1] - 1 {
            for x in 0..n[0] - 1 {
                let node = |c: usize| {
                    let k = corner(c);
                    ravel([x + k[0], y + k[1], z + k[2]], n)
                };
                let mut case = 0usize;
                for c in 0..8 {
                    if inside(phi[node(c)]) {
                        case |= 1 << c;
                    }
                }
                if case == 0 || case == 255 {
                    continue;
                }
                for lp in &tab[case] {
                    let mut ids = Vec::with_capacity(lp.len());
                    for &e in lp {
                        let (a, c) = ed[e as usize];
                        let n0 = node(c);
                        if vid[a][n0] == u32::MAX {
                            let n1 = node(c | (1 << a));
                            let (p0, p1) = (phi[n0], phi[n1]);
                            let t = if (p0 - p1).abs() > 1e-12 { (p0 / (p0 - p1)).clamp(0.0, 1.0) } else { 0.5 };
                            let k = unravel(n0, n);
                            let base = Vec3::new(k[0] as f32, k[1] as f32, k[2] as f32);
                            let p = origin + (base + with(Vec3::ZERO, a, t)) * h;
                            vid[a][n0] = positions.len() as u32;
                            positions.push(p);
                        }
                        ids.push(vid[a][n0]);
                    }
                    for k in 1..ids.len().saturating_sub(1) {
                        let t = [ids[0], ids[k], ids[k + 1]];
                        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                            faces.push(Face::new(t.to_vec()));
                        }
                    }
                }
            }
        }
    }
    let mut m = Mesh::new();
    m.positions = positions;
    m.faces = faces;
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_mesh_is_closed_and_outward() {
        let n = [24, 24, 24];
        let h = 0.1;
        let c = Vec3::splat(1.15);
        let r = 0.8;
        let phi: Vec<f32> = (0..n[0] * n[1] * n[2])
            .map(|i| {
                let k = unravel(i, n);
                (Vec3::new(k[0] as f32, k[1] as f32, k[2] as f32) * h - c).length() - r
            })
            .collect();
        let m = marching_cubes(&phi, n, Vec3::ZERO, h);
        let v = mesh_volume(&m);
        let exact = 4.0 / 3.0 * std::f32::consts::PI * r * r * r;
        assert!((v - exact).abs() / exact < 0.03, "volume {v} vs {exact}");
        // Every edge used twice, in opposite directions.
        let mut count = std::collections::HashMap::new();
        for f in &m.faces {
            for (a, b) in f.edges() {
                *count.entry((a, b)).or_insert(0) += 1;
            }
        }
        for (&(a, b), &k) in &count {
            assert_eq!(k, 1, "edge used twice in one direction");
            assert_eq!(count.get(&(b, a)), Some(&1), "open edge");
        }
    }

    #[test]
    fn every_case_closes() {
        for (case, loops) in table().iter().enumerate() {
            let crossings = edges().iter().filter(|&&(a, c)| ((case >> c) & 1) != ((case >> (c | (1 << a))) & 1)).count();
            let used: usize = loops.iter().map(|l| l.len()).sum();
            assert_eq!(used, crossings, "case {case}");
        }
    }
}
