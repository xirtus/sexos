// SPDX-License-Identifier: GPL-3.0-only
//! Modifiers that read other objects or edit vertex groups: Armature, Hook,
//! Lattice, Warp (`MOD_armature.cc`, `MOD_hook.cc`, `MOD_lattice.cc`,
//! `MOD_warp.cc`) and Vertex Weight Edit / Mix / Proximity
//! (`MOD_weightvg*.cc`), plus the vertex-group mask shared by the older
//! deform modifiers.
//!
//! Without a scene in the [`EvalContext`] (or when the referenced object is
//! missing or of the wrong kind) these modifiers leave the mesh unchanged,
//! like Blender does with an unset object field.
use super::{EvalContext, Modifier};
use crate::armature::DeformOptions;
use crate::math::{Mat4, Vec3};
use crate::mesh::Mesh;
use crate::scene::{ObjectData, Scene};

/// Falloff / mapping curves (Hook, Warp and the Vertex Weight modifiers
/// share Blender's numbering: 0 none, 1 curve, 2 sharp, 3 smooth, 4 root,
/// 5 linear, 6 constant, 7 sphere, 8 inverse square / random, 9 step).
/// Custom curves are not stored; `Curve` behaves like `Linear`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    None,
    Curve,
    Sharp,
    Smooth,
    Root,
    Linear,
    Constant,
    Sphere,
    InverseSquare,
    Random,
    Step,
}

impl Falloff {
    pub const NAMES: [&'static str; 11] = ["NONE", "CURVE", "SHARP", "SMOOTH", "ROOT", "LINEAR", "CONSTANT", "SPHERE", "INVERSE_SQUARE", "RANDOM", "STEP"];
    const ALL: [Falloff; 11] = [
        Falloff::None,
        Falloff::Curve,
        Falloff::Sharp,
        Falloff::Smooth,
        Falloff::Root,
        Falloff::Linear,
        Falloff::Constant,
        Falloff::Sphere,
        Falloff::InverseSquare,
        Falloff::Random,
        Falloff::Step,
    ];

    pub fn name(self) -> &'static str {
        Self::NAMES[Self::ALL.iter().position(|f| *f == self).unwrap_or(0)]
    }

    pub fn from_name(s: &str) -> Option<Falloff> {
        let s = s.to_ascii_uppercase();
        Self::NAMES.iter().position(|n| *n == s).map(|i| Self::ALL[i])
    }

    /// From a Blender DNA value. `weight_vg` selects the Vertex Weight
    /// numbering, where 8 is Random and 9 Step (Hook/Warp: 8 is Inverse
    /// Square).
    pub fn from_blender(v: i64, weight_vg: bool) -> Falloff {
        match v {
            1 => Falloff::Curve,
            2 => Falloff::Sharp,
            3 => Falloff::Smooth,
            4 => Falloff::Root,
            5 => Falloff::Linear,
            6 => Falloff::Constant,
            7 => Falloff::Sphere,
            8 if weight_vg => Falloff::Random,
            8 => Falloff::InverseSquare,
            9 => Falloff::Step,
            _ => Falloff::None,
        }
    }

    /// Shape `x` (0..1, 1 = full effect). `None`, `Linear` and `Curve` are
    /// the identity; `Random` uses `seed`.
    pub fn shape(self, x: f32, seed: u32) -> f32 {
        match self {
            Falloff::None | Falloff::Linear | Falloff::Curve => x,
            Falloff::Constant => 1.0,
            Falloff::Sharp => x * x,
            Falloff::Smooth => 3.0 * x * x - 2.0 * x * x * x,
            Falloff::Root => x.max(0.0).sqrt(),
            Falloff::Sphere => (2.0 * x - x * x).max(0.0).sqrt(),
            Falloff::InverseSquare => x * (2.0 - x),
            Falloff::Random => hash01(seed) * x,
            Falloff::Step => {
                if x >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

fn hash01(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x >> 8) as f32 / (1u32 << 24) as f32
}

/// Vertex Weight Proximity distance: to the target's origin or its geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProximityMode {
    Object,
    Geometry,
}

impl ProximityMode {
    pub fn name(self) -> &'static str {
        match self {
            ProximityMode::Object => "OBJECT",
            ProximityMode::Geometry => "GEOMETRY",
        }
    }
    pub fn from_name(s: &str) -> Option<ProximityMode> {
        match s.to_ascii_uppercase().as_str() {
            "OBJECT" => Some(ProximityMode::Object),
            "GEOMETRY" => Some(ProximityMode::Geometry),
            _ => None,
        }
    }
}

/// Vertex Weight Mix operation (A = A op B).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightMixMode {
    Set,
    Add,
    Subtract,
    Multiply,
    Divide,
    Difference,
    Average,
    Min,
    Max,
}

impl WeightMixMode {
    pub const NAMES: [&'static str; 9] = ["SET", "ADD", "SUB", "MUL", "DIV", "DIF", "AVG", "MIN", "MAX"];
    const ALL: [WeightMixMode; 9] = [
        WeightMixMode::Set,
        WeightMixMode::Add,
        WeightMixMode::Subtract,
        WeightMixMode::Multiply,
        WeightMixMode::Divide,
        WeightMixMode::Difference,
        WeightMixMode::Average,
        WeightMixMode::Min,
        WeightMixMode::Max,
    ];
    pub fn name(self) -> &'static str {
        Self::NAMES[Self::ALL.iter().position(|f| *f == self).unwrap_or(0)]
    }
    pub fn from_name(s: &str) -> Option<WeightMixMode> {
        let s = s.to_ascii_uppercase();
        Self::NAMES.iter().position(|n| *n == s).map(|i| Self::ALL[i])
    }
    /// Blender's `MOD_WVG_MIX_*` (1 set .. 7 avg, 8 min, 9 max).
    pub fn from_blender(v: i64) -> WeightMixMode {
        match v {
            2 => WeightMixMode::Add,
            3 => WeightMixMode::Subtract,
            4 => WeightMixMode::Multiply,
            5 => WeightMixMode::Divide,
            6 => WeightMixMode::Difference,
            7 => WeightMixMode::Average,
            8 => WeightMixMode::Min,
            9 => WeightMixMode::Max,
            _ => WeightMixMode::Set,
        }
    }
    fn mix(self, a: f32, b: f32) -> f32 {
        match self {
            WeightMixMode::Set => b,
            WeightMixMode::Add => a + b,
            WeightMixMode::Subtract => a - b,
            WeightMixMode::Multiply => a * b,
            WeightMixMode::Divide => {
                if b != 0.0 {
                    a / b
                } else if a == 0.0 {
                    0.0
                } else {
                    a.signum() * f32::MAX
                }
            }
            WeightMixMode::Difference => (a - b).abs(),
            WeightMixMode::Average => (a + b) * 0.5,
            WeightMixMode::Min => a.min(b),
            WeightMixMode::Max => a.max(b),
        }
    }
}

/// Which vertices Vertex Weight Mix affects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WeightMixSet {
    All,
    A,
    B,
    Or,
    And,
}

impl WeightMixSet {
    pub const NAMES: [&'static str; 5] = ["ALL", "A", "B", "OR", "AND"];
    const ALL: [WeightMixSet; 5] = [WeightMixSet::All, WeightMixSet::A, WeightMixSet::B, WeightMixSet::Or, WeightMixSet::And];
    pub fn name(self) -> &'static str {
        Self::NAMES[Self::ALL.iter().position(|f| *f == self).unwrap_or(0)]
    }
    pub fn from_name(s: &str) -> Option<WeightMixSet> {
        let s = s.to_ascii_uppercase();
        Self::NAMES.iter().position(|n| *n == s).map(|i| Self::ALL[i])
    }
    /// Blender's `MOD_WVG_SET_*` (1 all, 2 A, 3 B, 4 or, 5 and).
    pub fn from_blender(v: i64) -> WeightMixSet {
        match v {
            1 => WeightMixSet::All,
            3 => WeightMixSet::B,
            4 => WeightMixSet::Or,
            5 => WeightMixSet::And,
            _ => WeightMixSet::A,
        }
    }
}

/// Per-vertex modifier weights from an optional vertex group: `None` when
/// no group is named or the mesh lacks it (full effect, like Blender).
fn group_factors(m: &Mesh, group: Option<&str>, invert: bool) -> Option<Vec<f32>> {
    let g = m.vertex_group_index(group.filter(|g| !g.is_empty())?)?;
    Some((0..m.positions.len()).map(|v| if invert { 1.0 - m.weight(v, g) } else { m.weight(v, g) }).collect())
}

/// Blend `out` back towards `m` by the weights of group `g` (deform
/// modifiers with a vertex group but no group logic of their own).
pub(super) fn mask_by_group(m: &Mesh, out: &mut Mesh, g: &str, invert: bool) {
    if out.positions.len() != m.positions.len() {
        return;
    }
    if let Some(w) = group_factors(m, Some(g), invert) {
        for ((p, &o), w) in out.positions.iter_mut().zip(&m.positions).zip(w) {
            *p = o.lerp(*p, w);
        }
    }
}

fn scene_and_object<'a>(cx: &EvalContext<'a>, name: &str) -> Option<(&'a Scene, usize)> {
    let scene = cx.scene?;
    if name.is_empty() || cx.owner >= scene.objects.len() {
        return None;
    }
    let j = scene.find_object(name)?;
    (j != cx.owner).then_some((scene, j))
}

/// World matrix of object `j`, or of its bone `bone` when given.
fn target_matrix(scene: &Scene, j: usize, bone: &str) -> Mat4 {
    let world = scene.world_matrix(j);
    if !bone.is_empty() {
        if let Some(b) = scene.armature_of(j).and_then(|a| a.find(bone)) {
            if let Some(p) = scene.pose_matrices(j).get(b) {
                return world * *p;
            }
        }
    }
    world
}

pub(super) fn armature(md: &Modifier, m: &Mesh, cx: &EvalContext<'_>, prev: Option<&[Vec3]>) -> Mesh {
    let Modifier::Armature { object, use_vertex_groups, use_bone_envelopes, use_deform_preserve_volume, vertex_group, invert_vertex_group, .. } = md else {
        return m.clone();
    };
    let Some((scene, j)) = scene_and_object(cx, object) else { return m.clone() };
    let Some(arm) = scene.armature_of(j) else { return m.clone() };
    let pose = scene.pose_matrices(j);
    let premat = scene.world_matrix(j).inverse() * scene.world_matrix(cx.owner);
    let opts = DeformOptions {
        use_vertex_groups: *use_vertex_groups,
        use_bone_envelopes: *use_bone_envelopes,
        preserve_volume: *use_deform_preserve_volume,
        vertex_group: vertex_group.clone(),
        invert_vertex_group: *invert_vertex_group,
    };
    let prev = prev.filter(|p| p.len() == m.positions.len());
    let mut out = m.clone();
    out.positions = crate::armature::deform_positions(m, &m.positions, arm, &pose, &premat, &opts, prev);
    out
}

pub(super) fn hook(md: &Modifier, m: &Mesh, cx: &EvalContext<'_>) -> Mesh {
    let Modifier::Hook {
        object,
        subtarget,
        falloff_type,
        falloff_radius,
        center,
        matrix_inverse,
        strength,
        use_falloff_uniform,
        vertex_indices,
        vertex_group,
        invert_vertex_group,
    } = md
    else {
        return m.clone();
    };
    let Some((scene, j)) = scene_and_object(cx, object) else { return m.clone() };
    let owner_world = scene.world_matrix(cx.owner);
    let mat = owner_world.inverse() * target_matrix(scene, j, subtarget) * *matrix_inverse;
    let radius = if *falloff_type == Falloff::None { 0.0 } else { *falloff_radius };
    let use_falloff = radius * radius != 0.0;
    let uniform = use_falloff_uniform.then(|| owner_world.mat3());
    let cent = Vec3::new(center[0], center[1], center[2]);
    let cent = uniform.as_ref().map_or(cent, |u| u.mul_vec(cent));
    let factors = group_factors(m, vertex_group.as_deref(), *invert_vertex_group);
    let n = m.positions.len();
    // Hooked vertices: the stored indices, else the vertex group's members.
    let verts: Vec<usize> = if !vertex_indices.is_empty() {
        let mut v: Vec<usize> = vertex_indices.iter().map(|&i| i as usize).filter(|&i| i < n).collect();
        v.sort_unstable();
        v.dedup();
        v
    } else if let Some(g) = vertex_group.as_deref().and_then(|g| m.vertex_group_index(g)) {
        (0..n).filter(|&v| *invert_vertex_group || m.weight_opt(v, g).is_some()).collect()
    } else {
        Vec::new()
    };
    let mut out = m.clone();
    for v in verts {
        let co = m.positions[v];
        let mut fac = if use_falloff {
            let p = uniform.as_ref().map_or(co, |u| u.mul_vec(co));
            let len_sq = (p - cent).length_squared();
            if len_sq > radius * radius {
                0.0
            } else if len_sq > 0.0 {
                let x = 1.0 - len_sq.sqrt() / radius;
                falloff_type.shape(x, v as u32) * strength
            } else {
                *strength
            }
        } else {
            *strength
        };
        if let Some(w) = &factors {
            fac *= w[v];
        }
        if fac != 0.0 {
            out.positions[v] = co.lerp(mat.transform_point(co), fac);
        }
    }
    out
}

pub(super) fn lattice(md: &Modifier, m: &Mesh, cx: &EvalContext<'_>) -> Mesh {
    let Modifier::Lattice { object, strength, vertex_group, invert_vertex_group } = md else { return m.clone() };
    let Some((scene, j)) = scene_and_object(cx, object) else { return m.clone() };
    let ObjectData::Lattice(li) = scene.objects[j].data else { return m.clone() };
    let Some(lat) = scene.lattices.get(li) else { return m.clone() };
    let latmat = scene.world_matrix(j).inverse() * scene.world_matrix(cx.owner);
    let factors = group_factors(m, vertex_group.as_deref(), *invert_vertex_group);
    let mut out = m.clone();
    lat.deform(&mut out.positions, &latmat, *strength, factors.as_deref());
    out
}

pub(super) fn warp(md: &Modifier, m: &Mesh, cx: &EvalContext<'_>) -> Mesh {
    let Modifier::Warp { object_from, object_to, strength, falloff_type, falloff_radius, use_volume_preserve, vertex_group, invert_vertex_group } = md else {
        return m.clone();
    };
    let (Some((scene, from)), Some((_, to))) = (scene_and_object(cx, object_from), scene_and_object(cx, object_to)) else { return m.clone() };
    let obinv = scene.world_matrix(cx.owner).inverse();
    let mat_from = obinv * scene.world_matrix(from);
    let mat_to = obinv * scene.world_matrix(to);
    let mat_from_inv = mat_from.inverse();
    let mat_final = mat_from_inv * mat_to;
    // Volume preserving blends towards the matrix with unit scale.
    let mat_unit = Mat4::IDENTITY;
    let centre = mat_from.translation_part();
    let factors = group_factors(m, vertex_group.as_deref(), *invert_vertex_group);
    let r = *falloff_radius;
    let mut out = m.clone();
    for (v, co) in out.positions.iter_mut().enumerate() {
        let mut fac = if *falloff_type == Falloff::None {
            1.0
        } else {
            let d2 = (*co - centre).length_squared();
            if r <= 0.0 || d2 >= r * r {
                continue;
            }
            let x = (r - d2.sqrt()) / r;
            if x == 0.0 {
                continue;
            }
            falloff_type.shape(x, v as u32)
        };
        fac *= strength * factors.as_ref().map_or(1.0, |w| w[v]);
        if fac == 0.0 {
            continue;
        }
        let local = mat_from_inv.transform_point(*co);
        let moved = if fac == 1.0 {
            mat_final.transform_point(local)
        } else if *use_volume_preserve {
            mat_unit.interpolate(&mat_final, fac).transform_point(local)
        } else {
            local.lerp(mat_final.transform_point(local), fac)
        };
        *co = mat_from.transform_point(moved);
    }
    out
}

/// `weightvg_do_mask` with a constant influence.
fn mask(org: f32, new: f32, constant: f32) -> f32 {
    if constant >= 1.0 {
        new
    } else if constant <= 0.0 {
        org
    } else {
        org * (1.0 - constant) + new * constant
    }
}

/// `weightvg_update_vg`: write `w` (per vertex in `verts`) into group `g`,
/// clamping (or normalising) and adding/removing by thresholds.
#[allow(clippy::too_many_arguments)]
fn update_group(out: &mut Mesh, g: usize, verts: &[usize], w: &mut [f32], add: Option<f32>, remove: Option<f32>, normalize: bool) {
    if normalize && !w.is_empty() {
        let (lo, hi) = w.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
        let d = hi - lo;
        if d > 0.0 {
            w.iter_mut().for_each(|x| *x = (*x - lo) / d);
        }
    }
    for (&v, &x) in verts.iter().zip(w.iter()) {
        let x = x.clamp(0.0, 1.0);
        let present = out.weight_opt(v, g).is_some();
        if remove.is_some_and(|t| x < t) {
            if present {
                out.remove_weight(v, g);
            }
        } else if present || add.is_some_and(|t| x > t) {
            out.set_weight(v, g, x);
        }
    }
}

fn map_falloff(w: f32, falloff: Falloff, invert: bool, v: usize) -> f32 {
    let mut x = if falloff == Falloff::None { w } else { falloff.shape(w.clamp(0.0, 1.0), v as u32) };
    if invert {
        x = 1.0 - x;
    }
    x
}

/// Distance from `p` to segment `a`-`b`.
fn seg_dist(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let l = ab.dot(ab);
    let t = if l > 0.0 { ((p - a).dot(ab) / l).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).length()
}

pub(super) fn weights(md: &Modifier, m: &Mesh, cx: &EvalContext<'_>) -> Mesh {
    let mut out = m.clone();
    let n = m.positions.len();
    match md {
        Modifier::VertexWeightEdit { vertex_group, default_weight, use_add, add_threshold, use_remove, remove_threshold, falloff_type, invert_falloff, normalize, mask_constant } => {
            let Some(g) = m.vertex_group_index(vertex_group) else { return out };
            let verts: Vec<usize> = (0..n).collect();
            let mut w: Vec<f32> = verts
                .iter()
                .map(|&v| {
                    let org = m.weight_opt(v, g).unwrap_or(*default_weight);
                    mask(org, map_falloff(org, *falloff_type, *invert_falloff, v), *mask_constant)
                })
                .collect();
            update_group(&mut out, g, &verts, &mut w, use_add.then_some(*add_threshold), use_remove.then_some(*remove_threshold), *normalize);
        }
        Modifier::VertexWeightMix {
            vertex_group_a,
            vertex_group_b,
            default_weight_a,
            default_weight_b,
            mix_mode,
            mix_set,
            invert_vertex_group_a,
            invert_vertex_group_b,
            normalize,
            mask_constant,
        } => {
            let Some(ga) = m.vertex_group_index(vertex_group_a) else { return out };
            let gb = if vertex_group_b.is_empty() {
                None
            } else {
                match m.vertex_group_index(vertex_group_b) {
                    Some(g) => Some(g),
                    None => return out,
                }
            };
            let in_a = |v: usize| m.weight_opt(v, ga).is_some();
            let in_b = |v: usize| gb.is_some_and(|g| m.weight_opt(v, g).is_some());
            let verts: Vec<usize> = (0..n)
                .filter(|&v| match mix_set {
                    WeightMixSet::All => true,
                    WeightMixSet::A => in_a(v),
                    WeightMixSet::B => in_b(v),
                    WeightMixSet::Or => in_a(v) || in_b(v),
                    WeightMixSet::And => in_a(v) && in_b(v),
                })
                .collect();
            let mut w: Vec<f32> = verts
                .iter()
                .map(|&v| {
                    let mut a = m.weight_opt(v, ga).unwrap_or(*default_weight_a);
                    if *invert_vertex_group_a {
                        a = 1.0 - a;
                    }
                    let mut b = gb.and_then(|g| m.weight_opt(v, g)).unwrap_or(*default_weight_b);
                    if *invert_vertex_group_b {
                        b = 1.0 - b;
                    }
                    mask(a, mix_mode.mix(a, b), *mask_constant)
                })
                .collect();
            update_group(&mut out, ga, &verts, &mut w, Some(f32::MIN), None, *normalize);
        }
        Modifier::VertexWeightProximity {
            vertex_group,
            target,
            proximity_mode,
            use_proximity_geometry,
            min_dist,
            max_dist,
            falloff_type,
            invert_falloff,
            normalize,
            mask_constant,
        } => {
            let Some(g) = m.vertex_group_index(vertex_group) else { return out };
            let Some((scene, j)) = scene_and_object(cx, target) else { return out };
            let verts: Vec<usize> = (0..n).filter(|&v| m.weight_opt(v, g).is_some()).collect();
            if verts.is_empty() {
                return out;
            }
            // Distances are measured in world space.
            let owner_world = scene.world_matrix(cx.owner);
            let world: Vec<Vec3> = verts.iter().map(|&v| owner_world.transform_point(m.positions[v])).collect();
            let target_mesh = match (proximity_mode, &scene.objects[j].data) {
                (ProximityMode::Geometry, ObjectData::Mesh(_)) if use_proximity_geometry.iter().any(|b| *b) => scene.evaluated_mesh(j).map(|mut t| {
                    t.transform(&scene.world_matrix(j));
                    t
                }),
                _ => None,
            };
            let dists: Vec<f32> = match &target_mesh {
                Some(t) => {
                    let bvh = use_proximity_geometry[2].then(|| super::bvh::TriBvh::new(t));
                    let edges = if use_proximity_geometry[1] { t.edges() } else { Vec::new() };
                    world
                        .iter()
                        .map(|&p| {
                            let mut d = f32::MAX;
                            if use_proximity_geometry[0] {
                                for &q in &t.positions {
                                    d = d.min((p - q).length());
                                }
                            }
                            for &[a, b] in &edges {
                                d = d.min(seg_dist(p, t.positions[a as usize], t.positions[b as usize]));
                            }
                            if let Some(h) = bvh.as_ref().and_then(|b| b.nearest(p)) {
                                d = d.min(h.dist);
                            }
                            d
                        })
                        .collect()
                }
                None => {
                    let o = scene.world_matrix(j).translation_part();
                    world.iter().map(|&p| (p - o).length()).collect()
                }
            };
            let (lo, hi) = (*min_dist, *max_dist);
            let mut w: Vec<f32> = verts
                .iter()
                .zip(&dists)
                .map(|(&v, &d)| {
                    let x = if lo == hi {
                        if d <= lo {
                            0.0
                        } else {
                            1.0
                        }
                    } else if hi > lo {
                        ((d - lo) / (hi - lo)).clamp(0.0, 1.0)
                    } else {
                        ((lo - d) / (lo - hi)).clamp(0.0, 1.0)
                    };
                    let org = m.weight(v, g);
                    mask(org, map_falloff(x, *falloff_type, *invert_falloff, v), *mask_constant)
                })
                .collect();
            update_group(&mut out, g, &verts, &mut w, None, None, *normalize);
        }
        _ => {}
    }
    out
}
