// SPDX-License-Identifier: GPL-3.0-only
//! Dynamic Paint: canvases whose vertices are painted, dented or rippled by
//! brush objects (Blender's Dynamic Paint modifier, Vertex format).
//!
//! A canvas keeps per vertex a colour with alpha, a wetness, a value
//! (displace depth or weight) and, for Waves, a height velocity. Each frame
//! (split into Substeps + 1 steps) every brush (a Dynamic Paint modifier
//! with Brush on, in the Brush Collection when one is set) touches the
//! vertices it reaches: Mesh Volume inside the brush mesh, Proximity within
//! Distance of its surface (Smooth or Constant falloff), both, or Object
//! Center within Distance of its origin. Paint mixes the brush colour in by
//! Alpha and wets; wet paint Spreads to neighbours, Drips down along
//! gravity, Shrinks, Dries and Dissolves. Displace keeps the deepest
//! penetration (or adds it, Incremental); Weight keeps the strongest
//! influence; Waves runs a damped wave equation on the mesh's edges with
//! brushes as obstacles, depth changes or forces.
//!
//! The modifier writes the Paintmap and Wetmap as vertex colour attributes,
//! the Weight surface as a vertex group, and Displace/Waves as offsets along
//! the vertex normals. Not done: the Image Sequence format (it simulates on
//! vertices too), particle brushes, velocity ramps, smudging.
use super::rigid::shape::TriMesh;
use super::settings::{DpFalloff, DpInitColorType, DpPaintSource, DpSurfaceType, DpWaveType, DynamicPaintSettings};
use super::PhysicsModifier;
use crate::math::{Aabb, Vec3};
use crate::mesh::{AttrDomain, ColorAttribute};
use crate::mesh::Mesh;
use crate::modifiers::Modifier;
use crate::scene::{ObjectData, Scene};
use std::sync::Arc;

/// A canvas's state at one frame, per vertex of its mesh.
#[derive(Clone, Debug)]
pub struct PaintFrame {
    pub color: Arc<Vec<[f32; 4]>>,
    pub wet: Arc<Vec<f32>>,
    /// Displace depth, weight or wave height.
    pub value: Arc<Vec<f32>>,
    /// Wave height velocity.
    pub vel: Arc<Vec<f32>>,
}

impl PaintFrame {
    pub fn bytes(&self) -> usize {
        self.color.len() * 16 + (self.wet.len() + self.value.len() + self.vel.len()) * 4
    }
}

pub fn key(obj: &str, slot: usize) -> String {
    format!("{obj}/dynamicpaint{slot}")
}

/// The canvas in world space: positions, normals and edge neighbours.
struct Canvas {
    pos: Vec<Vec3>,
    nbrs: Vec<Vec<u32>>,
    /// Vertices on an open edge (used by once).
    border: Vec<bool>,
}

fn canvas(scene: &Scene, obj: usize) -> Option<Canvas> {
    let ObjectData::Mesh(mi) = scene.objects.get(obj)?.data else { return None };
    let m = scene.meshes.get(mi)?;
    let wm = scene.world_matrix(obj);
    let pos: Vec<Vec3> = m.positions.iter().map(|&p| wm.transform_point(p)).collect();
    let mut nbrs = vec![Vec::new(); pos.len()];
    let mut uses = std::collections::BTreeMap::new();
    for f in &m.faces {
        for (a, b) in f.edges() {
            let k = (a.min(b), a.max(b));
            *uses.entry(k).or_insert(0) += 1;
        }
    }
    let mut border = vec![false; pos.len()];
    for (&(a, b), &n) in &uses {
        nbrs[a as usize].push(b);
        nbrs[b as usize].push(a);
        if n == 1 {
            border[a as usize] = true;
            border[b as usize] = true;
        }
    }
    Some(Canvas { pos, nbrs, border })
}

struct Brush {
    s: DynamicPaintSettings,
    mesh: Option<TriMesh>,
    origin: Vec3,
}

fn brushes(scene: &Scene, canvas_obj: usize, s: &DynamicPaintSettings) -> Vec<Brush> {
    let coll = if s.brush_collection.is_empty() { None } else { scene.collections.iter().position(|c| c.name == s.brush_collection) };
    let mut out = Vec::new();
    for (i, o) in scene.objects.iter().enumerate() {
        if i == canvas_obj || coll.is_some_and(|c| o.collection != c) {
            continue;
        }
        for m in &o.modifiers {
            let Modifier::Physics(PhysicsModifier::DynamicPaint(b)) = m else { continue };
            if !b.use_brush {
                continue;
            }
            let mesh = super::fluid::world_trimesh(scene, i);
            out.push(Brush { s: (**b).clone(), mesh, origin: scene.world_matrix(i).transform_point(Vec3::ZERO) });
        }
    }
    out
}

fn inside(mesh: &TriMesh, p: Vec3) -> bool {
    let d = Vec3::new(1.0, 0.000_37, 0.000_19);
    let mut hits = 0;
    for t in 0..mesh.tris.len() {
        let [a, b, c] = mesh.tri(t);
        let (e1, e2) = (b - a, c - a);
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = p - a;
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
    hits % 2 == 1
}

fn surface_distance(mesh: &TriMesh, p: Vec3, reach: f32, scratch: &mut Vec<usize>) -> f32 {
    scratch.clear();
    mesh.query(&Aabb { min: p - Vec3::splat(reach), max: p + Vec3::splat(reach) }, scratch);
    scratch
        .iter()
        .map(|&t| {
            let [a, b, c] = mesh.tri(t);
            super::collision::closest_on_triangle(p, a, b, c).distance(p)
        })
        .fold(f32::INFINITY, f32::min)
}

/// A brush's influence at `p` (0..1) and its depth (penetration for
/// volumes, closeness for proximity).
fn influence(b: &Brush, p: Vec3, radius_scale: f32, scratch: &mut Vec<usize>) -> (f32, f32) {
    let dist = b.s.paint_distance * radius_scale;
    let falloff = |d: f32| -> f32 {
        if d > dist || dist <= 0.0 {
            return 0.0;
        }
        match b.s.proximity_falloff {
            DpFalloff::Constant => 1.0,
            DpFalloff::Smooth => {
                let t = 1.0 - d / dist;
                t * t * (3.0 - 2.0 * t)
            }
        }
    };
    match b.s.paint_source {
        DpPaintSource::Point => {
            let d = p.distance(b.origin);
            (falloff(d), (dist - d).max(0.0))
        }
        src => {
            let Some(m) = &b.mesh else { return (0.0, 0.0) };
            let bb = m.bounds();
            let reach = if src == DpPaintSource::Volume { 0.0 } else { dist };
            if p.x < bb.min.x - reach || p.y < bb.min.y - reach || p.z < bb.min.z - reach || p.x > bb.max.x + reach || p.y > bb.max.y + reach || p.z > bb.max.z + reach {
                return (0.0, 0.0);
            }
            let vol = src != DpPaintSource::Distance && inside(m, p);
            if vol && src == DpPaintSource::Volume {
                let depth = surface_distance(m, p, bb.size().max_elem(), scratch);
                return (1.0, depth.min(bb.size().max_elem()));
            }
            if src == DpPaintSource::Volume {
                return (0.0, 0.0);
            }
            let d = surface_distance(m, p, dist.max(1e-4), scratch);
            let mut f = if d.is_finite() { falloff(d) } else { 0.0 };
            if src == DpPaintSource::VolumeDistance {
                if b.s.invert_proximity && d.is_finite() && d <= dist {
                    f = 1.0 - f;
                }
                if vol {
                    f = if b.s.use_negative_volume { 0.0 } else { 1.0 };
                }
            }
            (f, if vol { d.min(dist) } else { (dist - d).max(0.0) })
        }
    }
}

/// A fresh canvas at the start frame (then painted once).
pub fn initial(scene: &Scene, obj: usize, s: &DynamicPaintSettings) -> Option<PaintFrame> {
    let c = canvas(scene, obj)?;
    let n = c.pos.len();
    let col = if s.init_color_type == DpInitColorType::Color { [s.init_color[0], s.init_color[1], s.init_color[2], 1.0] } else { [0.0; 4] };
    let f = PaintFrame { color: Arc::new(vec![col; n]), wet: Arc::new(vec![0.0; n]), value: Arc::new(vec![0.0; n]), vel: Arc::new(vec![0.0; n]) };
    Some(paint(scene, obj, s, &c, &f, 1))
}

/// Advance a canvas one frame.
pub fn step(scene: &Scene, obj: usize, s: &DynamicPaintSettings, prev: &PaintFrame) -> Option<PaintFrame> {
    let c = canvas(scene, obj)?;
    if c.pos.len() != prev.color.len() {
        return None;
    }
    Some(paint(scene, obj, s, &c, prev, s.frame_substeps.max(0) as usize + 1))
}

fn paint(scene: &Scene, obj: usize, s: &DynamicPaintSettings, c: &Canvas, prev: &PaintFrame, substeps: usize) -> PaintFrame {
    let n = c.pos.len();
    let mut color = (*prev.color).clone();
    let mut wet = (*prev.wet).clone();
    let mut value = (*prev.value).clone();
    let mut vel = (*prev.vel).clone();
    let bs = brushes(scene, obj, s);
    let mut scratch = Vec::new();
    // Influences are where the brushes are this frame.
    let infl: Vec<Vec<(f32, f32)>> = bs.iter().map(|b| c.pos.iter().map(|&p| influence(b, p, s.brush_radius_scale, &mut scratch)).collect()).collect();
    let gs = &scene.physics.gravity;
    let gdir = if gs.use_gravity { Vec3::from_array(gs.gravity).normalize() } else { Vec3::new(0.0, 0.0, -1.0) };
    let frac = 1.0 / substeps as f32;
    for _ in 0..substeps {
        for (b, inf) in bs.iter().zip(&infl) {
            for v in 0..n {
                let (f, depth) = inf[v];
                let f = f * s.brush_influence_scale;
                if f <= 0.0 {
                    continue;
                }
                match s.surface_type {
                    DpSurfaceType::Paint => {
                        let a = (b.s.paint_alpha * f).clamp(0.0, 1.0);
                        let c0 = color[v];
                        if b.s.use_paint_erase {
                            color[v][3] = (c0[3] - a).max(0.0);
                        } else {
                            let na = if b.s.use_absolute_alpha { c0[3].max(a) } else { c0[3] + a * (1.0 - c0[3]) };
                            let mixw = if na > 0.0 { a / na.max(1e-6) } else { 1.0 };
                            for k in 0..3 {
                                color[v][k] = c0[k] + (b.s.paint_color[k] - c0[k]) * mixw.min(1.0);
                            }
                            color[v][3] = na;
                            wet[v] = wet[v].max(b.s.paint_wetness * f);
                        }
                    }
                    DpSurfaceType::Displace => {
                        let d = depth * f;
                        value[v] = if s.use_incremental_displace { value[v] + d * frac } else { value[v].max(d) };
                        if s.depth_clamp > 0.0 {
                            value[v] = value[v].min(s.depth_clamp);
                        }
                    }
                    DpSurfaceType::Weight => value[v] = value[v].max(f.min(1.0)),
                    DpSurfaceType::Wave => {
                        // A volume brush presses in by its depth below the surface.
                        let d = if depth > 0.0 { depth } else { f };
                        let mut h = -b.s.wave_factor * d;
                        if b.s.wave_clamp > 0.0 {
                            h = h.clamp(-b.s.wave_clamp, b.s.wave_clamp);
                        }
                        match b.s.wave_type {
                            DpWaveType::Depth => {
                                if h < value[v] {
                                    value[v] = h;
                                    vel[v] = 0.0;
                                }
                            }
                            DpWaveType::Change => value[v] += h * frac,
                            DpWaveType::Force => vel[v] += h * frac,
                            DpWaveType::Reflect => {
                                value[v] = 0.0;
                                vel[v] = 0.0;
                            }
                        }
                    }
                }
            }
        }
        match s.surface_type {
            DpSurfaceType::Paint => effects(s, c, &mut color, &mut wet, gdir, frac),
            DpSurfaceType::Wave => waves(s, c, &mut value, &mut vel, scene.fps.max(1.0), frac),
            _ => {}
        }
    }
    // Drying and dissolving, per frame.
    if s.surface_type == DpSurfaceType::Paint && s.use_drying {
        let k = 1.0 / s.dry_speed.max(1) as f32;
        for w in &mut wet {
            *w = if s.use_dry_log { *w * (1.0 - k) } else { (*w - k).max(0.0) };
        }
    }
    if s.use_dissolve && s.surface_type != DpSurfaceType::Wave {
        let k = 1.0 / s.dissolve_speed.max(1) as f32;
        let fade = |x: f32| if s.use_dissolve_log { x * (1.0 - k) } else { (x - k).max(0.0) };
        if s.surface_type == DpSurfaceType::Paint {
            for col in &mut color {
                col[3] = fade(col[3]);
            }
        } else {
            for x in &mut value {
                *x = fade(*x);
            }
        }
    }
    PaintFrame { color: Arc::new(color), wet: Arc::new(wet), value: Arc::new(value), vel: Arc::new(vel) }
}

/// Spread, Drip and Shrink of wet paint.
fn effects(s: &DynamicPaintSettings, c: &Canvas, color: &mut [[f32; 4]], wet: &mut [f32], gdir: Vec3, frac: f32) {
    let n = color.len();
    if s.use_spread {
        let prev = color.to_vec();
        let pw = wet.to_vec();
        let k = (0.2 * s.spread_speed * frac).min(0.5);
        for v in 0..n {
            let nb = &c.nbrs[v];
            if nb.is_empty() {
                continue;
            }
            let mut acc = [0.0f32; 4];
            let mut wsum = 0.0;
            for &u in nb {
                let w = pw[u as usize];
                if w <= 0.0 {
                    continue;
                }
                for i in 0..4 {
                    acc[i] += (prev[u as usize][i] - prev[v][i]) * w;
                }
                wsum += w;
            }
            if wsum <= 0.0 {
                continue;
            }
            let t = k / nb.len() as f32;
            for i in 0..3 {
                color[v][i] += acc[i] * t * s.color_spread_speed;
            }
            color[v][3] = (color[v][3] + acc[3].max(0.0) * t).min(1.0);
            wet[v] = wet[v].max((wsum / nb.len() as f32) * k);
        }
    }
    if s.use_drip {
        let prev = color.to_vec();
        let pw = wet.to_vec();
        let k = (0.05 * s.drip_velocity.abs() * frac).min(0.5);
        for v in 0..n {
            if pw[v] <= 0.01 || prev[v][3] <= 0.0 {
                continue;
            }
            // The neighbour most downhill.
            let mut best = None;
            let mut bd = 0.0;
            for &u in &c.nbrs[v] {
                let d = (c.pos[u as usize] - c.pos[v]).normalize().dot(gdir);
                if d > bd {
                    bd = d;
                    best = Some(u as usize);
                }
            }
            let Some(u) = best else { continue };
            let t = k * pw[v] * bd;
            let moved = prev[v][3] * t;
            color[v][3] -= moved;
            let na = (color[u][3] + moved).min(1.0);
            for i in 0..3 {
                color[u][i] = color[u][i] + (prev[v][i] - color[u][i]) * (moved / na.max(1e-6)).min(1.0);
            }
            color[u][3] = na;
            wet[u] = wet[u].max(pw[v] * 0.9);
        }
    }
    if s.use_shrink {
        let prev = color.to_vec();
        let k = (0.1 * s.shrink_speed * frac).min(1.0);
        for v in 0..n {
            let lo = c.nbrs[v].iter().map(|&u| prev[u as usize][3]).fold(prev[v][3], f32::min);
            color[v][3] -= (prev[v][3] - lo) * k;
        }
    }
}

/// A damped wave equation on the mesh's edges.
fn waves(s: &DynamicPaintSettings, c: &Canvas, h: &mut [f32], vel: &mut [f32], fps: f32, frac: f32) {
    let dt = s.wave_timescale * frac / fps * 25.0;
    let speed2 = s.wave_speed * s.wave_speed * 60.0;
    // Explicit steps stay stable while dt * omega < 2 (omega from the
    // stiffest mode: twice the edge Laplacian plus the spring); keep a margin.
    let omega = (2.0 * speed2 + s.wave_spring.max(0.0) * 10.0).sqrt();
    let steps = ((dt * omega).ceil() as usize).clamp(1, 256);
    let dt = dt / steps as f32;
    for _ in 0..steps {
        let prev = h.to_vec();
        for v in 0..h.len() {
            let nb = &c.nbrs[v];
            if nb.is_empty() {
                continue;
            }
            if c.border[v] && !s.use_wave_open_border {
                vel[v] = 0.0;
                continue;
            }
            let avg = nb.iter().map(|&u| prev[u as usize]).sum::<f32>() / nb.len() as f32;
            let acc = speed2 * (avg - prev[v]) - s.wave_spring * 10.0 * prev[v];
            vel[v] = (vel[v] + acc * dt) * (1.0 - s.wave_damping).powf(dt * 25.0);
        }
        for v in 0..h.len() {
            h[v] += vel[v] * dt;
            if s.wave_smoothness > 0.0 {
                let lim = 1.0 / s.wave_smoothness;
                h[v] = h[v].clamp(-lim, lim);
            }
        }
    }
}

/// The canvas as the modifier outputs it at the scene's frame.
pub fn modifier_eval(m: &Mesh, scene: &Scene, key: &str, s: &DynamicPaintSettings) -> Mesh {
    if !s.use_canvas || scene.frame_current < s.frame_start {
        return m.clone();
    }
    let Some(f) = scene.physics.caches.frame_at(key, scene.frame_current.min(s.frame_end)) else { return m.clone() };
    let super::cache::SimFrame::Paint(p) = &*f else { return m.clone() };
    if p.color.len() != m.positions.len() {
        return m.clone();
    }
    let mut out = m.clone();
    match s.surface_type {
        DpSurfaceType::Paint => {
            let mut set = |name: &str, data: Vec<[f32; 4]>| {
                out.color_attributes.retain(|a| a.name != name);
                out.color_attributes.push(ColorAttribute { name: name.to_string(), domain: AttrDomain::Point, data, byte: false });
            };
            if s.use_output_a {
                set(&s.output_name_a, p.color.to_vec());
            }
            if s.use_output_b {
                set(&s.output_name_b, p.wet.iter().map(|&w| [w, w, w, 1.0]).collect());
            }
        }
        DpSurfaceType::Weight => {
            let name = s.weight_group().to_string();
            let g = match out.vertex_groups.iter().position(|x| *x == name) {
                Some(g) => g,
                None => {
                    out.vertex_groups.push(name);
                    out.vertex_groups.len() - 1
                }
            } as u32;
            out.weights.resize(out.positions.len(), Vec::new());
            for (w, &x) in out.weights.iter_mut().zip(p.value.iter()) {
                w.retain(|e| e.0 != g);
                if x > 0.0 {
                    w.push((g, x));
                }
            }
        }
        DpSurfaceType::Displace | DpSurfaceType::Wave => {
            let nrm = m.vertex_normals();
            let sign = if s.surface_type == DpSurfaceType::Displace { -s.displace_factor } else { 1.0 };
            for (i, p0) in out.positions.iter_mut().enumerate() {
                *p0 += nrm[i] * (p.value[i] * sign);
            }
        }
    }
    out
}
