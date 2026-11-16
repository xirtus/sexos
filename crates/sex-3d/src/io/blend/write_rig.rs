// SPDX-License-Identifier: GPL-3.0-only
//! Rigging data for the `.blend` writer: the object modifier stack
//! (`ModifierData` subtypes, the Nodes modifier's `IDProperty` inputs),
//! object and bone constraints (`bConstraint` with its type's data struct),
//! shape keys (`Key`/`KeyBlock`) and lattices (`Lattice`/`BPoint`). Values
//! are the inverse of the reader (`blend.rs`, `blend/rig.rs`,
//! `blend/geonodes.rs`); member names and flag bits are those of Blender
//! 4.2's `DNA_modifier_types.h`, `DNA_constraint_types.h`,
//! `DNA_key_types.h` and `DNA_lattice_types.h` (GPL-2+).
use super::export::{euler_mode, Ids};
use super::write::{Rec, V, W};
use crate::constraints::{Constraint, ConstraintKind as K, LimitDistanceMode, RotationMix, Space, StretchPlane, StretchVolume, Target, TrackAxis, TransformChannel, TransformMix, TransformationMix, VolumeMode};
use crate::lattice::Lattice;
use crate::math::{EulerOrder, Mat4};
use crate::mesh::{KeyInterpolation, Mesh};
use crate::modifiers::{BooleanOp, CastType, DecimateMode, Falloff, Modifier, ProximityMode, RemeshMode, ShrinkwrapMethod, SimpleDeformMode, WeightMixMode, WeightMixSet};
use crate::nodes::{CurveMapping, Value};
use crate::scene::Scene;

/// `eModifierMode_Realtime | eModifierMode_Render`.
const MODE_ON: i64 = 1 | 2;
/// `ID_ME`, a mesh key's `idroot`-like owner code.
const KEY_RELATIVE: i64 = 1;
const KEY_NORMAL: i64 = 0;
/// `CONSTRAINT_OFF`.
const CONSTRAINT_OFF: i64 = 1 << 9;
/// `CONSTRAINT_IK_POS`, set by Blender's IK init.
const IK_POS: i64 = 1 << 5;

/// The DNA value whose reader mapping `from` gives `x`, trying 1.. first
/// (so enums where 0 is a fallback pick the real value), then 0.
pub(super) fn inv<T: PartialEq>(from: impl Fn(i64) -> T, x: T) -> i64 {
    (1..64).chain([0]).find(|&v| from(v) == x).unwrap_or(0)
}

fn bits(on: &[(bool, i64)]) -> i64 {
    on.iter().filter(|(b, _)| *b).fold(0, |a, (_, m)| a | m)
}

fn euler(o: Option<EulerOrder>) -> i64 {
    o.map_or(0, euler_mode)
}

fn space(s: Space) -> i64 {
    inv(Space::from_blender, s)
}

fn track(t: TrackAxis) -> i64 {
    // TRACK_X .. TRACK_nZ are 0..5.
    (0..6).find(|&v| TrackAxis::from_blender(v) == t).unwrap_or(1)
}

impl Ids {
    /// The address of the object named `name` (0 when there is none).
    pub(super) fn object(&self, name: &str) -> u64 {
        self.object_by_name.get(name).copied().unwrap_or(0)
    }
    pub(super) fn collection(&self, name: &str) -> u64 {
        self.collection_by_name.get(name).copied().unwrap_or(0)
    }
}

/// Modifier names made unique within one stack (`Mirror`, `Mirror.001`).
fn unique_names(stack: &[Modifier]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in stack {
        let base = m.name().to_string();
        let mut n = base.clone();
        let mut k = 1;
        while out.contains(&n) {
            n = format!("{base}.{k:03}");
            k += 1;
        }
        out.push(n);
    }
    out
}

/// What the physics modifiers keep on the object rather than in the stack.
#[derive(Default)]
pub(super) struct StackExtras {
    pub(super) modifiers: Vec<u64>,
    /// `Object.particlesystem` entries.
    pub(super) particle_systems: Vec<u64>,
    /// The `ParticleSystem` records behind them, written by the object.
    pub(super) pending_psys: Vec<(u64, Rec)>,
}

impl W {
    /// Write the modifier stack of object `oi` and link it; `extras`
    /// collects what the physics modifiers keep on the object.
    pub(super) fn modifier_stack(&mut self, scene: &Scene, oi: usize, ids: &Ids, extras: &mut StackExtras) {
        let o = &scene.objects[oi];
        let names = unique_names(&o.modifiers);
        let groups: Vec<String> = match o.data {
            crate::scene::ObjectData::Mesh(mi) => scene.meshes.get(mi).map(|m| m.vertex_groups.clone()).unwrap_or_default(),
            _ => Vec::new(),
        };
        let mut recs = Vec::new();
        for (k, m) in o.modifiers.iter().enumerate() {
            let Some(mut r) = self.modifier_rec(m, scene, oi, k, ids, &groups, extras) else { continue };
            self.s(&mut r, "modifier.name", &names[k]);
            self.i(&mut r, "modifier.mode", MODE_ON);
            self.i(&mut r, "modifier.ui_expand_flag", 1);
            // Stable, unique and >= 1 (BKE_modifiers_persistent_uid_init).
            self.i(&mut r, "modifier.persistent_uid", k as i64 + 1);
            recs.push(r);
        }
        if let Some(last) = recs.last_mut() {
            let f = self.get(last, "modifier.flag");
            self.i(last, "modifier.flag", f | 1 << 2); // eModifierFlag_Active
        }
        let addrs: Vec<u64> = recs.iter().map(|_| self.addr()).collect();
        for (k, r) in recs.iter_mut().enumerate() {
            self.p(r, "modifier.next", addrs.get(k + 1).copied().unwrap_or(0));
            self.p(r, "modifier.prev", if k > 0 { addrs[k - 1] } else { 0 });
        }
        for (r, a) in recs.iter().zip(&addrs) {
            self.put(b"DATA", *a, std::slice::from_ref(r));
        }
        extras.modifiers = addrs;
    }

    /// A default linear `CurveMapping` (what Blender's modifier init adds).
    fn falloff_curve(&mut self) -> u64 {
        self.curve_mapping(&CurveMapping { curves: vec![vec![[0.0, 0.0], [1.0, 1.0]]] }, 1, [0.0, 0.0, 1.0, 1.0])
    }

    /// One modifier as its DNA struct with `modifier.type` set.
    #[allow(clippy::too_many_arguments)]
    fn modifier_rec(&mut self, m: &Modifier, scene: &Scene, oi: usize, k: usize, ids: &Ids, groups: &[String], extras: &mut StackExtras) -> Option<Rec> {
        let ob = |n: &str| ids.object(n);
        let vg = |g: &Option<String>| g.clone().unwrap_or_default();
        let (sname, ty) = struct_of(m)?;
        let mut r = self.rec(sname);
        self.i(&mut r, "modifier.type", ty);
        match m {
            Modifier::Mirror { x, y, z, merge, merge_distance } => {
                let keep = self.get(&r, "flag") & !(7 << 3 | 1 << 7);
                self.i(&mut r, "flag", keep | bits(&[(*x, 1 << 3), (*y, 1 << 4), (*z, 1 << 5), (!*merge, 1 << 7)]));
                self.f(&mut r, "tolerance", &[*merge_distance]);
            }
            Modifier::Subdivision { levels, simple } => {
                self.i(&mut r, "levels", *levels as i64);
                let rl = (*levels as i64).max(self.get(&r, "renderLevels"));
                self.i(&mut r, "renderLevels", rl);
                self.i(&mut r, "subdivType", *simple as i64);
            }
            Modifier::Array { count, relative, constant, merge } => {
                self.i(&mut r, "count", *count as i64);
                self.f(&mut r, "scale", relative);
                self.f(&mut r, "offset", constant);
                let rel = relative.iter().any(|v| *v != 0.0);
                let con = constant.iter().any(|v| *v != 0.0);
                self.i(&mut r, "offset_type", bits(&[(con, 1), (rel, 2)]));
                self.i(&mut r, "flags", *merge as i64);
            }
            Modifier::Solidify { thickness, offset } => {
                self.f(&mut r, "offset", &[*thickness]);
                self.f(&mut r, "offset_fac", &[*offset]);
            }
            Modifier::Bevel { width, segments, angle } => {
                self.f(&mut r, "value", &[*width]);
                self.i(&mut r, "res", *segments as i64);
                self.i(&mut r, "affect_type", 1); // MOD_BEVEL_AFFECT_EDGES
                if *angle > 0.0 {
                    self.i(&mut r, "lim_flags", 1 << 3); // MOD_BEVEL_ANGLE
                    self.f(&mut r, "bevel_angle", &[*angle]);
                } else {
                    self.i(&mut r, "lim_flags", 0);
                }
            }
            Modifier::Triangulate => {}
            Modifier::Weld { distance } => self.f(&mut r, "merge_dist", &[*distance]),
            Modifier::Displace { strength, vertex_group, invert_vertex_group } => {
                // Without a texture Blender moves by (1 - midlevel) * strength.
                self.f(&mut r, "midlevel", &[0.0]);
                self.f(&mut r, "strength", &[*strength]);
                self.i(&mut r, "direction", 3); // MOD_DISP_DIR_NOR
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
                self.i(&mut r, "flag", *invert_vertex_group as i64);
            }
            Modifier::Smooth { factor, iterations, vertex_group, invert_vertex_group } => {
                self.f(&mut r, "fac", &[*factor]);
                self.i(&mut r, "repeat", (*iterations).min(32767) as i64);
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
                let f = self.get(&r, "flag") & !1;
                self.i(&mut r, "flag", f | *invert_vertex_group as i64);
            }
            Modifier::Decimate { mode, ratio, iterations, angle, triangulate } => {
                self.i(&mut r, "mode", match mode {
                    DecimateMode::Collapse => 0,
                    DecimateMode::Unsubdivide => 1,
                    DecimateMode::Planar => 2,
                });
                self.f(&mut r, "percent", &[*ratio]);
                self.i(&mut r, "iter", (*iterations).min(32767) as i64);
                self.f(&mut r, "angle", &[*angle]);
                let f = self.get(&r, "flag") & !(1 << 1);
                self.i(&mut r, "flag", f | bits(&[(*triangulate, 1 << 1)]));
            }
            Modifier::Remesh { mode, voxel_size, octree_depth, scale, smooth_shade } => {
                self.i(&mut r, "mode", match mode {
                    RemeshMode::Blocks => 0,
                    RemeshMode::Smooth => 1,
                    RemeshMode::Voxel => 3,
                });
                self.f(&mut r, "voxel_size", &[*voxel_size]);
                self.i(&mut r, "depth", (*octree_depth).min(127) as i64);
                self.f(&mut r, "scale", &[*scale]);
                let f = self.get(&r, "flag") & !(1 << 1);
                self.i(&mut r, "flag", f | bits(&[(*smooth_shade, 1 << 1)]));
            }
            Modifier::Screw { axis, angle, steps, screw_offset, iterations, merge, merge_distance, flip, smooth_shade } => {
                self.i(&mut r, "axis", *axis as i64);
                self.f(&mut r, "angle", &[*angle]);
                self.i(&mut r, "steps", *steps as i64);
                self.i(&mut r, "render_steps", *steps as i64);
                self.f(&mut r, "screw_ofs", &[*screw_offset]);
                self.i(&mut r, "iter", *iterations as i64);
                self.f(&mut r, "merge_dist", &[*merge_distance]);
                let f = self.get(&r, "flag") & !(1 | 1 << 5 | 1 << 8);
                self.i(&mut r, "flag", f | bits(&[(*flip, 1), (*smooth_shade, 1 << 5), (*merge, 1 << 8)]));
            }
            Modifier::Wireframe { thickness, offset, even, relative, boundary, replace } => {
                self.f(&mut r, "offset", &[*thickness]);
                self.f(&mut r, "offset_fac", &[*offset]);
                let f = self.get(&r, "flag") & !(0b11110);
                self.i(&mut r, "flag", f | bits(&[(*replace, 1 << 1), (*boundary, 1 << 2), (*even, 1 << 3), (*relative, 1 << 4)]));
            }
            Modifier::EdgeSplit { angle, use_angle } => {
                self.f(&mut r, "split_angle", &[*angle]);
                let f = self.get(&r, "flags") & !(1 << 1);
                self.i(&mut r, "flags", f | bits(&[(*use_angle, 1 << 1)]));
            }
            Modifier::Build { start, length, reverse, random, seed } => {
                self.f(&mut r, "start", &[*start]);
                self.f(&mut r, "length", &[*length]);
                self.i(&mut r, "flag", bits(&[(*random, 1), (*reverse, 1 << 1)]));
                self.i(&mut r, "seed", *seed as i64);
            }
            Modifier::Boolean { object, operation } => {
                self.p(&mut r, "object", ob(object));
                self.i(&mut r, "operation", match operation {
                    BooleanOp::Intersect => 0,
                    BooleanOp::Union => 1,
                    BooleanOp::Difference => 2,
                });
            }
            Modifier::SimpleDeform { mode, axis, factor, limits, lock, vertex_group, invert_vertex_group } => {
                self.i(&mut r, "mode", match mode {
                    SimpleDeformMode::Twist => 1,
                    SimpleDeformMode::Bend => 2,
                    SimpleDeformMode::Taper => 3,
                    SimpleDeformMode::Stretch => 4,
                });
                self.i(&mut r, "deform_axis", *axis as i64);
                self.f(&mut r, "factor", &[*factor]);
                self.f(&mut r, "limit", limits);
                self.i(&mut r, "axis", bits(&[(lock[0], 1), (lock[1], 2), (lock[2], 4)]));
                self.s(&mut r, "vgroup_name", &vg(vertex_group));
                self.i(&mut r, "flag", *invert_vertex_group as i64);
            }
            Modifier::Cast { kind, factor, radius, size, axes, size_from_radius, vertex_group, invert_vertex_group } => {
                self.i(&mut r, "type", match kind {
                    CastType::Sphere => 0,
                    CastType::Cylinder => 1,
                    CastType::Cuboid => 2,
                });
                self.f(&mut r, "fac", &[*factor]);
                self.f(&mut r, "radius", &[*radius]);
                self.f(&mut r, "size", &[*size]);
                let f = self.get(&r, "flag") & !(1 | 0b1110 | 1 << 5);
                self.i(&mut r, "flag", f | bits(&[(*invert_vertex_group, 1), (axes[0], 1 << 1), (axes[1], 1 << 2), (axes[2], 1 << 3), (*size_from_radius, 1 << 5)]));
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
            }
            Modifier::Wave { x, y, cyclic, normals, falloff, height, width, narrowness, speed, time_offset, lifetime, damping, start, vertex_group, invert_vertex_group } => {
                let f = self.get(&r, "flag") & !0b11111;
                self.i(&mut r, "flag", f | bits(&[(*invert_vertex_group, 1), (*x, 1 << 1), (*y, 1 << 2), (*cyclic, 1 << 3), (*normals, 1 << 4)]));
                for (k, v) in [("falloff", falloff), ("height", height), ("width", width), ("narrow", narrowness), ("speed", speed), ("timeoffs", time_offset), ("lifetime", lifetime), ("damp", damping)] {
                    self.f(&mut r, k, &[*v]);
                }
                self.f(&mut r, "startx", &[start[0]]);
                self.f(&mut r, "starty", &[start[1]]);
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
            }
            Modifier::Shrinkwrap { target, method, offset, axes, positive, negative, vertex_group, invert_vertex_group } => {
                self.p(&mut r, "target", ob(target));
                self.i(&mut r, "shrinkType", match method {
                    ShrinkwrapMethod::NearestSurfacePoint => 0,
                    ShrinkwrapMethod::Project => 1,
                    ShrinkwrapMethod::NearestVertex => 2,
                });
                self.f(&mut r, "keepDist", &[*offset]);
                self.i(&mut r, "projAxis", bits(&[(axes[0], 1), (axes[1], 2), (axes[2], 4)]));
                let f = self.get(&r, "shrinkOpts") & !(1 | 2 | 1 << 6);
                self.i(&mut r, "shrinkOpts", f | bits(&[(*positive, 1), (*negative, 2), (*invert_vertex_group, 1 << 6)]));
                self.s(&mut r, "vgroup_name", &vg(vertex_group));
            }
            Modifier::LaplacianSmooth { iterations, factor, border, axes, preserve_volume, normalized, vertex_group, invert_vertex_group } => {
                self.i(&mut r, "repeat", (*iterations).min(32767) as i64);
                self.f(&mut r, "lambda", &[*factor]);
                self.f(&mut r, "lambda_border", &[*border]);
                self.i(&mut r, "flag", bits(&[(axes[0], 1 << 1), (axes[1], 1 << 2), (axes[2], 1 << 3), (*preserve_volume, 1 << 4), (*normalized, 1 << 5), (*invert_vertex_group, 1 << 6)]));
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
            }
            Modifier::CorrectiveSmooth { factor, iterations, scale, length_weighted, only_smooth, vertex_group, invert_vertex_group } => {
                self.f(&mut r, "lambda", &[*factor]);
                self.i(&mut r, "repeat", (*iterations).min(32767) as i64);
                self.f(&mut r, "scale", &[*scale]);
                self.i(&mut r, "smooth_type", *length_weighted as i64);
                let f = self.get(&r, "flag") & !3;
                self.i(&mut r, "flag", f | bits(&[(*invert_vertex_group, 1), (*only_smooth, 1 << 1)]));
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
            }
            Modifier::WeightedNormal { keep_sharp } => {
                let f = self.get(&r, "flag") & !1;
                self.i(&mut r, "flag", f | *keep_sharp as i64);
            }
            Modifier::Armature { object, use_vertex_groups, use_bone_envelopes, use_deform_preserve_volume, use_multi_modifier, vertex_group, invert_vertex_group } => {
                self.p(&mut r, "object", ob(object));
                self.i(&mut r, "deformflag", bits(&[(*use_vertex_groups, 1), (*use_bone_envelopes, 2), (*use_deform_preserve_volume, 4), (*invert_vertex_group, 16)]));
                self.i(&mut r, "multi", *use_multi_modifier as i64);
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
            }
            Modifier::Hook { object, subtarget, falloff_type, falloff_radius, center, matrix_inverse, strength, use_falloff_uniform, vertex_indices, vertex_group, invert_vertex_group } => {
                self.p(&mut r, "object", ob(object));
                self.s(&mut r, "subtarget", subtarget);
                self.i(&mut r, "falloff_type", inv(|v| Falloff::from_blender(v, false), *falloff_type));
                self.f(&mut r, "falloff", &[*falloff_radius]);
                self.f(&mut r, "cent", center);
                self.f(&mut r, "parentinv", &matrix_inverse.to_cols_array());
                self.f(&mut r, "force", &[*strength]);
                self.i(&mut r, "flag", bits(&[(*use_falloff_uniform, 1), (*invert_vertex_group, 2)]));
                self.s(&mut r, "name", &vg(vertex_group));
                if !vertex_indices.is_empty() {
                    let a = self.raw(vertex_indices.iter().flat_map(|v| (*v as i32).to_le_bytes()).collect());
                    self.p(&mut r, "indexar", a);
                    self.i(&mut r, "totindex", vertex_indices.len() as i64);
                }
                let c = self.falloff_curve();
                self.p(&mut r, "curfalloff", c);
            }
            Modifier::Lattice { object, strength, vertex_group, invert_vertex_group } => {
                self.p(&mut r, "object", ob(object));
                self.f(&mut r, "strength", &[*strength]);
                self.s(&mut r, "name", &vg(vertex_group));
                self.i(&mut r, "flag", *invert_vertex_group as i64);
            }
            Modifier::Warp { object_from, object_to, strength, falloff_type, falloff_radius, use_volume_preserve, vertex_group, invert_vertex_group } => {
                self.p(&mut r, "object_from", ob(object_from));
                self.p(&mut r, "object_to", ob(object_to));
                self.f(&mut r, "strength", &[*strength]);
                self.i(&mut r, "falloff_type", inv(|v| Falloff::from_blender(v, false), *falloff_type));
                self.f(&mut r, "falloff_radius", &[*falloff_radius]);
                self.i(&mut r, "flag", bits(&[(*use_volume_preserve, 1), (*invert_vertex_group, 2)]));
                self.s(&mut r, "defgrp_name", &vg(vertex_group));
                let c = self.falloff_curve();
                self.p(&mut r, "curfalloff", c);
            }
            Modifier::VertexWeightEdit { vertex_group, default_weight, use_add, add_threshold, use_remove, remove_threshold, falloff_type, invert_falloff, normalize, mask_constant } => {
                self.s(&mut r, "defgrp_name", vertex_group);
                self.f(&mut r, "default_weight", &[*default_weight]);
                self.f(&mut r, "add_threshold", &[*add_threshold]);
                self.f(&mut r, "rem_threshold", &[*remove_threshold]);
                self.i(&mut r, "falloff_type", inv(|v| Falloff::from_blender(v, true), *falloff_type));
                self.i(&mut r, "edit_flags", bits(&[(*normalize, 1), (*invert_falloff, 1 << 1), (*use_add, 1 << 3), (*use_remove, 1 << 4)]));
                self.f(&mut r, "mask_constant", &[*mask_constant]);
                let c = self.falloff_curve();
                self.p(&mut r, "cmap_curve", c);
            }
            Modifier::VertexWeightMix { vertex_group_a, vertex_group_b, default_weight_a, default_weight_b, mix_mode, mix_set, invert_vertex_group_a, invert_vertex_group_b, normalize, mask_constant } => {
                self.s(&mut r, "defgrp_name_a", vertex_group_a);
                self.s(&mut r, "defgrp_name_b", vertex_group_b);
                self.f(&mut r, "default_weight_a", &[*default_weight_a]);
                self.f(&mut r, "default_weight_b", &[*default_weight_b]);
                self.i(&mut r, "mix_mode", inv(WeightMixMode::from_blender, *mix_mode));
                self.i(&mut r, "mix_set", inv(WeightMixSet::from_blender, *mix_set));
                self.i(&mut r, "flag", bits(&[(*normalize, 2), (*invert_vertex_group_a, 4), (*invert_vertex_group_b, 8)]));
                self.f(&mut r, "mask_constant", &[*mask_constant]);
            }
            Modifier::VertexWeightProximity { vertex_group, target, proximity_mode, use_proximity_geometry, min_dist, max_dist, falloff_type, invert_falloff, normalize, mask_constant } => {
                self.s(&mut r, "defgrp_name", vertex_group);
                self.p(&mut r, "proximity_ob_target", ob(target));
                self.i(&mut r, "proximity_mode", match proximity_mode {
                    ProximityMode::Object => 1,
                    ProximityMode::Geometry => 2,
                });
                let g = use_proximity_geometry;
                self.i(&mut r, "proximity_flags", bits(&[(g[0], 1), (g[1], 2), (g[2], 4), (*invert_falloff, 16), (*normalize, 32)]));
                self.f(&mut r, "min_dist", &[*min_dist]);
                self.f(&mut r, "max_dist", &[*max_dist]);
                self.i(&mut r, "falloff_type", inv(|v| Falloff::from_blender(v, true), *falloff_type));
                self.f(&mut r, "mask_constant", &[*mask_constant]);
                let c = self.falloff_curve();
                self.p(&mut r, "cmap_curve", c);
            }
            Modifier::Nodes { group, inputs } => {
                let gi = scene.node_groups.iter().position(|g| &g.name == group);
                self.p(&mut r, "node_group", gi.and_then(|i| ids.groups.get(i).copied()).unwrap_or(0));
                let tree = gi.map(|i| &scene.node_groups[i]);
                let props = self.nodes_properties(tree, inputs);
                self.p(&mut r, "settings.properties", props);
            }
            Modifier::Physics(p) => self.physics_modifier(&mut r, p, scene, oi, k, ids, groups, extras),
            Modifier::Multires { .. } => return None,
        }
        Some(r)
    }

    /// The Nodes modifier's input values: an `IDP_GROUP` with one property
    /// per group input, named by its interface identifier (`Socket_<i>`).
    fn nodes_properties(&mut self, tree: Option<&crate::nodes::NodeTree>, inputs: &[(String, Value)]) -> u64 {
        let mut props = Vec::new();
        if let Some(t) = tree {
            for (name, v) in inputs {
                let Some(i) = t.inputs.iter().position(|s| &s.name == name) else { continue };
                if let Some(p) = self.id_property(&format!("Socket_{i}"), v) {
                    props.push(p);
                }
            }
        }
        let addrs: Vec<u64> = props.iter().map(|_| self.addr()).collect();
        let mut g = self.rec("IDProperty");
        self.i(&mut g, "type", 6); // IDP_GROUP
        self.s(&mut g, "name", "Nodes Modifier Settings");
        self.listbase(&mut g, "data.group", &addrs);
        self.i(&mut g, "len", props.len() as i64);
        let a = self.data(&[g]);
        self.chain(props, &addrs);
        a
    }

    /// One `IDProperty` holding a socket value (the reader's `id_property`).
    fn id_property(&mut self, name: &str, v: &Value) -> Option<Rec> {
        let mut p = self.rec("IDProperty");
        self.s(&mut p, "name", name);
        // IDP_FLAG_OVERRIDABLE_LIBRARY | IDP_FLAG_STATIC_TYPE, as MOD_nodes adds them.
        self.i(&mut p, "flag", 1 | 1 << 4);
        let floats = |w: &mut W, p: &mut Rec, xs: &[f32]| {
            let a = w.raw(xs.iter().flat_map(|x| x.to_le_bytes()).collect());
            w.i(p, "type", 5); // IDP_ARRAY
            w.i(p, "subtype", 2); // IDP_FLOAT
            w.p(p, "data.pointer", a);
            w.i(p, "len", xs.len() as i64);
            w.i(p, "totallen", xs.len() as i64);
        };
        match v {
            Value::Float(x) => {
                self.i(&mut p, "type", 2);
                self.set(&mut p, "data.val", V::I(&[x.to_bits() as i32 as i64]));
            }
            Value::Int(x) => {
                self.i(&mut p, "type", 1);
                self.i(&mut p, "data.val", *x as i64);
            }
            Value::Bool(b) => {
                self.i(&mut p, "type", 10);
                self.i(&mut p, "data.val", *b as i64);
            }
            Value::Vector(x) => floats(self, &mut p, x),
            Value::Color(x) => floats(self, &mut p, x),
            Value::String(s) => {
                let a = self.string(s);
                self.i(&mut p, "type", 0);
                self.p(&mut p, "data.pointer", a);
                self.i(&mut p, "len", s.len() as i64 + 1);
                self.i(&mut p, "totallen", s.len() as i64 + 1);
            }
            Value::Id(_) | Value::None => return None,
        }
        Some(p)
    }

    // ---------------------------------------------------------- Constraints

    /// A constraint list; returns the block addresses for its `ListBase`.
    pub(super) fn constraints(&mut self, list: &[Constraint], ids: &Ids) -> Vec<u64> {
        let mut recs = Vec::new();
        for (k, c) in list.iter().enumerate() {
            let (ty, data) = self.constraint_data(&c.kind, ids);
            let mut r = self.rec("bConstraint");
            self.p(&mut r, "data", data);
            self.i(&mut r, "type", ty);
            let active = if k + 1 == list.len() { 1 << 4 } else { 0 }; // CONSTRAINT_ACTIVE
            self.i(&mut r, "flag", active | if c.mute { CONSTRAINT_OFF } else { 0 });
            self.i(&mut r, "ownspace", space(c.owner_space));
            self.i(&mut r, "tarspace", space(c.target_space));
            self.i(&mut r, "ui_expand_flag", 1);
            self.p(&mut r, "space_object", ids.object(&c.space_object));
            self.s(&mut r, "space_subtarget", &c.space_subtarget);
            self.s(&mut r, "name", &c.name);
            self.f(&mut r, "enforce", &[c.influence]);
            let ht = match &c.kind {
                K::Armature { .. } => 0.0,
                _ => c.target().map_or(0.0, |t| t.head_tail),
            };
            self.f(&mut r, "headtail", &[ht]);
            recs.push(r);
        }
        let addrs: Vec<u64> = recs.iter().map(|_| self.addr()).collect();
        self.chain(recs, &addrs);
        addrs
    }

    /// Set `tar`/`subtarget`.
    fn target(&mut self, r: &mut Rec, t: &Target, ids: &Ids) {
        self.p(r, "tar", ids.object(&t.object));
        self.s(r, "subtarget", &t.subtarget);
    }

    /// The constraint's `CONSTRAINT_TYPE_*` and its data block.
    fn constraint_data(&mut self, k: &K, ids: &Ids) -> (i64, u64) {
        let xyz = |a: &[bool; 3], m: [i64; 3]| bits(&[(a[0], m[0]), (a[1], m[1]), (a[2], m[2])]);
        let (ty, sname) = match k {
            K::ChildOf { .. } => (1, "bChildOfConstraint"),
            K::TrackTo { .. } => (2, "bTrackToConstraint"),
            K::Ik { .. } => (3, "bKinematicConstraint"),
            K::LimitRotation { .. } => (5, "bRotLimitConstraint"),
            K::LimitLocation { .. } => (6, "bLocLimitConstraint"),
            K::LimitScale { .. } => (7, "bSizeLimitConstraint"),
            K::CopyRotation { .. } => (8, "bRotateLikeConstraint"),
            K::CopyLocation { .. } => (9, "bLocateLikeConstraint"),
            K::CopyScale { .. } => (10, "bSizeLikeConstraint"),
            K::LockedTrack { .. } => (13, "bLockTrackConstraint"),
            K::LimitDistance { .. } => (14, "bDistLimitConstraint"),
            K::StretchTo { .. } => (15, "bStretchToConstraint"),
            K::Floor { .. } => (16, "bMinMaxConstraint"),
            K::Transformation { .. } => (19, "bTransformConstraint"),
            K::DampedTrack { .. } => (21, "bDampTrackConstraint"),
            K::CopyTransforms { .. } => (23, "bTransLikeConstraint"),
            K::MaintainVolume { .. } => (24, "bSameVolumeConstraint"),
            K::Armature { .. } => (30, "bArmatureConstraint"),
        };
        let mut d = self.rec(sname);
        match k {
            K::CopyLocation { target, use_xyz, invert, use_offset } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "flag", xyz(use_xyz, [1, 2, 4]) | xyz(invert, [16, 32, 64]) | bits(&[(*use_offset, 128)]));
            }
            K::CopyRotation { target, use_xyz, invert, mix, euler_order } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "flag", xyz(use_xyz, [1, 4, 16]) | xyz(invert, [2, 8, 32]));
                self.i(&mut d, "mix_mode", match mix {
                    RotationMix::Replace => 0,
                    RotationMix::Add => 2,
                    RotationMix::Before => 3,
                    RotationMix::After => 4,
                });
                self.i(&mut d, "euler_order", euler(*euler_order));
            }
            K::CopyScale { target, use_xyz, power, use_make_uniform, use_offset, use_add } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "flag", xyz(use_xyz, [1, 2, 4]) | bits(&[(*use_offset, 8), (*use_add, 16), (*use_make_uniform, 32)]));
                self.f(&mut d, "power", &[*power]);
            }
            K::CopyTransforms { target, mix } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "mix_mode", match mix {
                    TransformMix::Replace => 0,
                    TransformMix::Before => 1,
                    TransformMix::After => 2,
                    TransformMix::BeforeSplit => 3,
                    TransformMix::AfterSplit => 4,
                    TransformMix::BeforeFull => 5,
                    TransformMix::AfterFull => 6,
                });
            }
            K::LimitDistance { target, distance, mode } => {
                self.target(&mut d, target, ids);
                self.f(&mut d, "dist", &[*distance]);
                self.i(&mut d, "mode", match mode {
                    LimitDistanceMode::Inside => 0,
                    LimitDistanceMode::Outside => 1,
                    LimitDistanceMode::OnSurface => 2,
                });
            }
            K::LimitLocation { use_min, min, use_max, max } | K::LimitScale { use_min, min, use_max, max } => {
                self.i(&mut d, "flag", xyz(use_min, [1, 4, 16]) | xyz(use_max, [2, 8, 32]));
                self.limits(&mut d, min, max);
            }
            K::LimitRotation { use_limit, min, max, euler_order } => {
                self.i(&mut d, "flag", xyz(use_limit, [1, 2, 4]));
                self.limits(&mut d, min, max);
                self.i(&mut d, "euler_order", euler(*euler_order));
            }
            K::MaintainVolume { mode, free_axis, volume } => {
                self.i(&mut d, "mode", match mode {
                    VolumeMode::Strict => 0,
                    VolumeMode::Uniform => 1,
                    VolumeMode::SingleAxis => 2,
                });
                // The free axis lives in `flag` (SAMEVOL_X/Y/Z).
                self.i(&mut d, "flag", (*free_axis).min(2) as i64);
                self.f(&mut d, "volume", &[*volume]);
            }
            K::Transformation { target, map_from, map_to, from_min, from_max, to_min, to_max, map, use_motion_extrapolate, mix, to_euler_order } => {
                self.target(&mut d, target, ids);
                let chan = |c: &TransformChannel| match c {
                    TransformChannel::Location => (0, ""),
                    TransformChannel::Rotation => (1, "_rot"),
                    TransformChannel::Scale => (2, "_scale"),
                };
                let (from, fs) = chan(map_from);
                let (to, ts) = chan(map_to);
                self.i(&mut d, "from", from);
                self.i(&mut d, "to", to);
                self.f(&mut d, &format!("from_min{fs}"), from_min);
                self.f(&mut d, &format!("from_max{fs}"), from_max);
                self.f(&mut d, &format!("to_min{ts}"), to_min);
                self.f(&mut d, &format!("to_max{ts}"), to_max);
                self.set(&mut d, "map", V::I(&[map[0] as i64, map[1] as i64, map[2] as i64]));
                self.i(&mut d, "expo", *use_motion_extrapolate as i64);
                self.i(&mut d, "to_euler_order", euler(*to_euler_order));
                match map_to {
                    TransformChannel::Location => self.i(&mut d, "mix_mode_loc", (*mix == TransformationMix::Replace) as i64),
                    TransformChannel::Rotation => self.i(&mut d, "mix_mode_rot", match mix {
                        TransformationMix::Replace => 1,
                        TransformationMix::Before => 2,
                        TransformationMix::After => 3,
                        _ => 0,
                    }),
                    TransformChannel::Scale => self.i(&mut d, "mix_mode_scale", (*mix == TransformationMix::Multiply) as i64),
                }
            }
            K::ChildOf { target, use_location, use_rotation, use_scale, inverse } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "flag", xyz(use_location, [1, 2, 4]) | xyz(use_rotation, [8, 16, 32]) | xyz(use_scale, [64, 128, 256]));
                self.f(&mut d, "invmat", &inverse.to_cols_array());
            }
            K::TrackTo { target, track_axis, up_axis, use_target_z } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "reserved1", track(*track_axis));
                self.i(&mut d, "reserved2", *up_axis as i64);
                self.i(&mut d, "flags", *use_target_z as i64);
            }
            K::DampedTrack { target, track_axis } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "trackflag", track(*track_axis));
            }
            K::LockedTrack { target, track_axis, lock_axis } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "trackflag", track(*track_axis));
                self.i(&mut d, "lockflag", *lock_axis as i64);
            }
            K::StretchTo { target, rest_length, bulge, use_bulge_min, bulge_min, use_bulge_max, bulge_max, bulge_smooth, volume, keep_axis } => {
                self.target(&mut d, target, ids);
                self.f(&mut d, "orglength", &[*rest_length]);
                self.f(&mut d, "bulge", &[*bulge]);
                self.f(&mut d, "bulge_min", &[*bulge_min]);
                self.f(&mut d, "bulge_max", &[*bulge_max]);
                self.f(&mut d, "bulge_smooth", &[*bulge_smooth]);
                self.i(&mut d, "flag", bits(&[(*use_bulge_min, 1), (*use_bulge_max, 2)]));
                self.i(&mut d, "volmode", match volume {
                    StretchVolume::XZ => 0,
                    StretchVolume::X => 1,
                    StretchVolume::Z => 2,
                    StretchVolume::None => 3,
                });
                self.i(&mut d, "plane", match keep_axis {
                    StretchPlane::X => 0,
                    StretchPlane::SwingY => 1,
                    StretchPlane::Z => 2,
                });
            }
            K::Floor { target, floor_location, offset, use_rotation } => {
                self.target(&mut d, target, ids);
                self.i(&mut d, "minmaxflag", track(*floor_location));
                self.f(&mut d, "offset", &[*offset]);
                self.i(&mut d, "flag", bits(&[(*use_rotation, 4)]));
            }
            K::Armature { targets, use_deform_preserve_volume, use_bone_envelopes } => {
                self.i(&mut d, "flag", bits(&[(*use_deform_preserve_volume, 1), (*use_bone_envelopes, 2)]));
                let recs: Vec<Rec> = targets
                    .iter()
                    .map(|(t, w)| {
                        let mut ct = self.rec("bConstraintTarget");
                        self.target(&mut ct, t, ids);
                        self.f(&mut ct, "weight", &[*w]);
                        self.f(&mut ct, "matrix", &Mat4::IDENTITY.to_cols_array());
                        ct
                    })
                    .collect();
                let addrs: Vec<u64> = recs.iter().map(|_| self.addr()).collect();
                self.listbase(&mut d, "targets", &addrs);
                self.chain(recs, &addrs);
            }
            K::Ik { target, pole_target, pole_angle, chain_count, iterations, use_tail, use_stretch, weight } => {
                self.target(&mut d, target, ids);
                if let Some(p) = pole_target {
                    self.p(&mut d, "poletar", ids.object(&p.object));
                    self.s(&mut d, "polesubtarget", &p.subtarget);
                }
                self.f(&mut d, "poleangle", &[*pole_angle]);
                self.i(&mut d, "rootbone", (*chain_count).min(255) as i64);
                self.i(&mut d, "iterations", (*iterations).min(32767) as i64);
                self.i(&mut d, "flag", IK_POS | bits(&[(*use_tail, 1), (*use_stretch, 16)]));
                self.f(&mut d, "weight", &[*weight]);
                self.f(&mut d, "orientweight", &[1.0]);
                self.f(&mut d, "dist", &[1.0]);
            }
        }
        (ty, self.data(&[d]))
    }

    fn limits(&mut self, d: &mut Rec, min: &[f32; 3], max: &[f32; 3]) {
        for (k, a) in ["x", "y", "z"].iter().enumerate() {
            self.f(d, &format!("{a}min"), &[min[k]]);
            self.f(d, &format!("{a}max"), &[max[k]]);
        }
    }

    // ----------------------------------------------------------- Shape keys

    /// A mesh's `Key` ID (`KE`) at `addr`, owned by the mesh at `mesh`.
    pub(super) fn shape_key(&mut self, addr: u64, name: &str, m: &Mesh, mesh: u64) {
        self.start_id();
        let mut r = self.id("Key", "KE", name);
        let nv = m.positions.len();
        let blocks: Vec<u64> = m.shape_keys.iter().map(|_| self.addr()).collect();
        self.listbase(&mut r, "block", &blocks);
        self.p(&mut r, "refkey", blocks.first().copied().unwrap_or(0));
        self.p(&mut r, "from", mesh);
        self.i(&mut r, "totkey", blocks.len() as i64);
        self.i(&mut r, "elemsize", 12);
        self.i(&mut r, "type", if m.shape_key_settings.use_relative { KEY_RELATIVE } else { KEY_NORMAL });
        self.f(&mut r, "ctime", &[m.shape_key_settings.eval_time]);
        self.i(&mut r, "uidgen", blocks.len() as i64 + 1);
        self.put(b"KE", addr, &[r]);
        let mut recs = Vec::new();
        for (k, sk) in m.shape_keys.iter().enumerate() {
            let pos: Vec<u8> = (0..nv).flat_map(|v| sk.positions.get(v).copied().unwrap_or(m.positions[v]).to_array()).flat_map(f32::to_le_bytes).collect();
            let data = self.raw(pos);
            let mut b = self.rec("KeyBlock");
            self.s(&mut b, "name", &sk.name);
            self.p(&mut b, "data", data);
            self.i(&mut b, "totelem", nv as i64);
            self.f(&mut b, "curval", &[sk.value]);
            self.f(&mut b, "pos", &[sk.frame / 100.0]);
            self.f(&mut b, "slidermin", &[sk.slider_min]);
            self.f(&mut b, "slidermax", &[sk.slider_max]);
            self.i(&mut b, "relative", sk.relative_key.min(m.shape_keys.len().saturating_sub(1)) as i64);
            self.i(&mut b, "flag", sk.mute as i64);
            self.s(&mut b, "vgroup", sk.vertex_group.as_deref().unwrap_or(""));
            self.i(&mut b, "type", inv(KeyInterpolation::from_blender, sk.interpolation));
            self.i(&mut b, "uid", k as i64 + 1);
            recs.push(b);
        }
        self.chain(recs, &blocks);
    }

    // ------------------------------------------------------------- Lattice

    pub(super) fn lattice(&mut self, addr: u64, name: &str, l: &Lattice) {
        self.start_id();
        let mut r = self.id("Lattice", "LT", name);
        let res = [l.points_u, l.points_v, l.points_w];
        for (k, ax) in ["u", "v", "w"].iter().enumerate() {
            let n = res[k].max(1);
            self.i(&mut r, &format!("pnts{ax}"), n as i64);
            self.i(&mut r, &format!("opnts{ax}"), n as i64);
            // calc_lat_fudu.
            let (fu, du) = if n <= 1 { (0.0, 0.0) } else { (-0.5, 1.0 / (n - 1) as f32) };
            self.f(&mut r, &format!("f{ax}"), &[fu]);
            self.f(&mut r, &format!("d{ax}"), &[du]);
        }
        let ty = |i: KeyInterpolation| inv(KeyInterpolation::from_blender, i);
        self.i(&mut r, "typeu", ty(l.interpolation_u));
        self.i(&mut r, "typev", ty(l.interpolation_v));
        self.i(&mut r, "typew", ty(l.interpolation_w));
        let f = self.get(&r, "flag") & !2;
        self.i(&mut r, "flag", f | bits(&[(l.use_outside, 2)]));
        let n = l.point_count();
        let def = if n > 0 { self.addr() } else { 0 };
        self.p(&mut r, "def", def);
        self.put(b"LT", addr, &[r]);
        if def != 0 {
            let recs: Vec<Rec> = (0..n)
                .map(|i| {
                    let p = l.points.get(i).copied().unwrap_or_default();
                    let mut b = self.rec("BPoint");
                    self.f(&mut b, "vec", &[p.x, p.y, p.z, 1.0]);
                    self.f(&mut b, "weight", &[1.0]);
                    self.f(&mut b, "radius", &[1.0]);
                    b
                })
                .collect();
            self.put(b"DATA", def, &recs);
        }
    }
}

/// The DNA struct and `ModifierType` of a modifier (`None`: not written).
fn struct_of(m: &Modifier) -> Option<(&'static str, i64)> {
    use crate::physics::PhysicsModifier as P;
    Some(match m {
        Modifier::Subdivision { .. } => ("SubsurfModifierData", 1),
        Modifier::Lattice { .. } => ("LatticeModifierData", 2),
        Modifier::Build { .. } => ("BuildModifierData", 4),
        Modifier::Mirror { .. } => ("MirrorModifierData", 5),
        Modifier::Decimate { .. } => ("DecimateModifierData", 6),
        Modifier::Wave { .. } => ("WaveModifierData", 7),
        Modifier::Armature { .. } => ("ArmatureModifierData", 8),
        Modifier::Hook { .. } => ("HookModifierData", 9),
        Modifier::Physics(P::SoftBody(_)) => ("SoftbodyModifierData", 10),
        Modifier::Boolean { .. } => ("BooleanModifierData", 11),
        Modifier::Array { .. } => ("ArrayModifierData", 12),
        Modifier::EdgeSplit { .. } => ("EdgeSplitModifierData", 13),
        Modifier::Displace { .. } => ("DisplaceModifierData", 14),
        Modifier::Smooth { .. } => ("SmoothModifierData", 16),
        Modifier::Cast { .. } => ("CastModifierData", 17),
        Modifier::Physics(P::ParticleSystem(_)) => ("ParticleSystemModifierData", 19),
        Modifier::Physics(P::Cloth(_)) => ("ClothModifierData", 22),
        Modifier::Physics(P::Collision(_)) => ("CollisionModifierData", 23),
        Modifier::Bevel { .. } => ("BevelModifierData", 24),
        Modifier::Shrinkwrap { .. } => ("ShrinkwrapModifierData", 25),
        Modifier::SimpleDeform { .. } => ("SimpleDeformModifierData", 28),
        Modifier::Solidify { .. } => ("SolidifyModifierData", 33),
        Modifier::Screw { .. } => ("ScrewModifierData", 34),
        Modifier::Warp { .. } => ("WarpModifierData", 35),
        Modifier::VertexWeightEdit { .. } => ("WeightVGEditModifierData", 36),
        Modifier::VertexWeightMix { .. } => ("WeightVGMixModifierData", 37),
        Modifier::VertexWeightProximity { .. } => ("WeightVGProximityModifierData", 38),
        Modifier::Remesh { .. } => ("RemeshModifierData", 41),
        Modifier::LaplacianSmooth { .. } => ("LaplacianSmoothModifierData", 43),
        Modifier::Triangulate => ("TriangulateModifierData", 44),
        Modifier::Wireframe { .. } => ("WireframeModifierData", 48),
        Modifier::CorrectiveSmooth { .. } => ("CorrectiveSmoothModifierData", 51),
        Modifier::WeightedNormal { .. } => ("WeightedNormalModifierData", 54),
        Modifier::Weld { .. } => ("WeldModifierData", 55),
        Modifier::Physics(P::Fluid(_)) => ("FluidModifierData", 56),
        Modifier::Nodes { .. } => ("NodesModifierData", 57),
        Modifier::Multires { .. } => return None,
    })
}
