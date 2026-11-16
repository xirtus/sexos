// SPDX-License-Identifier: GPL-3.0-only
//! Rigging data of `.blend` files: vertex groups, shape keys, armatures,
//! poses, constraints, lattices and the rigging modifiers. The DNA member
//! names and flag values are listed in the parent module's docs.
use super::{count, typed, BlendFile, Layer, View};
use crate::armature::{mat3_vec_to_roll, Armature, Bone, InheritScale, PoseBone, RotationMode};
use crate::constraints::{
    Constraint, ConstraintKind, LimitDistanceMode, RotationMix, Space, StretchPlane, StretchVolume, Target, TrackAxis, TransformChannel, TransformMix, TransformationMix, VolumeMode,
};
use crate::lattice::Lattice;
use crate::math::{EulerOrder, Mat4, Quat, Vec3};
use crate::mesh::{KeyInterpolation, Mesh, ShapeKey, ShapeKeySettings};
use crate::modifiers::{Falloff, Modifier, ProximityMode, WeightMixMode, WeightMixSet};
use std::collections::{HashMap, HashSet};

/// `CD_MDEFORMVERT`: the deform-weight CustomData layer.
const CD_MDEFORMVERT: i64 = 2;
/// More weights than this on one vertex is corrupt data.
const MAX_WEIGHTS_PER_VERT: usize = 4096;

/// Names of a `ListBase` of `bDeformGroup` (`Mesh.vertex_group_names`, or
/// `Object.defbase` before 3.0).
pub(super) fn group_names(f: &BlendFile<'_>, lb: Option<View<'_, '_>>) -> Vec<String> {
    f.list(lb, &["bDeformGroup"]).iter().map(|g| g.string("name").unwrap_or_default()).collect()
}

/// Deform weights (`MDeformVert`/`MDeformWeight`) and shape keys (`Key`,
/// `KeyBlock`) of mesh `me`, added to `mesh`.
pub(super) fn mesh_extras(f: &BlendFile<'_>, me: &View<'_, '_>, vd: &[Layer], mesh: &mut Mesh) {
    let nv = mesh.positions.len();
    mesh.vertex_groups = group_names(f, me.sub("vertex_group_names"));
    let dvert = typed(vd, CD_MDEFORMVERT).map_or(me.ptr("dvert"), |l| l.data);
    if dvert != 0 {
        if let Some(dv) = f.view_array(dvert, &["MDeformVert"], nv) {
            mesh.weights = dv
                .iter()
                .map(|d| {
                    let n = count(d.num("totweight").unwrap_or(0.0)).min(MAX_WEIGHTS_PER_VERT);
                    f.view_array(d.ptr("dw"), &["MDeformWeight"], n)
                        .unwrap_or_default()
                        .iter()
                        .filter_map(|w| {
                            let g = w.num("def_nr")?;
                            let x = w.f32("weight")?;
                            (g >= 0.0 && x.is_finite()).then_some((g as u32, x))
                        })
                        .collect()
                })
                .collect();
            if mesh.weights.iter().all(|w| w.is_empty()) {
                mesh.weights.clear();
            }
        }
    }
    let Some(key) = f.view_ptr(me.ptr("key"), &["Key"]) else { return };
    let blocks = f.list(key.sub("block"), &["KeyBlock"]);
    for kb in &blocks {
        let n = count(kb.num("totelem").unwrap_or(0.0)).min(nv);
        let data = f.raw_array(kb.ptr("data"), 12, n).unwrap_or(&[]);
        let mut positions: Vec<Vec3> = (0..data.len() / 12).filter_map(|i| Some(Vec3::new(f.e.f32(data, i * 12)?, f.e.f32(data, i * 12 + 4)?, f.e.f32(data, i * 12 + 8)?))).collect();
        positions.extend_from_slice(&mesh.positions[positions.len()..]);
        let fl = |name: &str, d: f32| kb.f32(name).filter(|x| x.is_finite()).unwrap_or(d);
        let rel = kb.num("relative").unwrap_or(0.0);
        mesh.shape_keys.push(ShapeKey {
            name: kb.string("name").unwrap_or_default(),
            positions,
            value: fl("curval", 0.0),
            slider_min: fl("slidermin", 0.0),
            slider_max: fl("slidermax", 1.0),
            relative_key: if rel >= 0.0 && (rel as usize) < blocks.len() { rel as usize } else { 0 },
            vertex_group: kb.string("vgroup").filter(|g| !g.is_empty()),
            mute: kb.num("flag").unwrap_or(0.0) as i64 & 1 != 0,
            interpolation: KeyInterpolation::from_blender(kb.num("type").unwrap_or(0.0) as i64),
            frame: fl("pos", 0.0) * 100.0,
        });
    }
    if !mesh.shape_keys.is_empty() {
        mesh.shape_key_settings = ShapeKeySettings { use_relative: key.num("type").unwrap_or(1.0) as i64 == 1, eval_time: key.f32("ctime").filter(|x| x.is_finite()).unwrap_or(0.0) };
    }
}

/// `n` structs of a `ListBase`, with the old address of each.
fn list_ptrs<'f, 'a>(f: &'f BlendFile<'a>, lb: Option<View<'_, 'a>>, names: &[&str]) -> Vec<(u64, View<'f, 'a>)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut p = lb.map_or(0, |l| l.ptr("first"));
    while p != 0 && seen.insert(p) && out.len() < 100_000 {
        match f.view_ptr(p, names) {
            Some(v) => {
                out.push((p, v));
                p = v.ptr("next");
            }
            None => break,
        }
    }
    out
}

/// Armature data (`bArmature`): the `Bone` tree in armature space and the
/// bone collections (4.0+).
pub(super) fn armature(f: &BlendFile<'_>, ar: &View<'_, '_>) -> Armature {
    let mut arm = Armature::default();
    let mut by_ptr: HashMap<u64, usize> = HashMap::new();
    let mut stack: Vec<(Option<View<'_, '_>>, Option<usize>)> = vec![(ar.sub("bonebase"), None)];
    let mut visited = HashSet::new();
    while let Some((lb, parent)) = stack.pop() {
        for (ptr, b) in list_ptrs(f, lb, &["Bone"]) {
            if !visited.insert(ptr) || arm.bones.len() >= 100_000 {
                continue;
            }
            let arm_mat = b.vec::<16>("arm_mat").map(Mat4::from_cols_array);
            let head = b.vec3("arm_head").or_else(|| arm_mat.map(|m| m.translation_part())).unwrap_or(Vec3::ZERO);
            let length = b.f32("length").unwrap_or(1.0);
            let tail = b.vec3("arm_tail").or_else(|| arm_mat.map(|m| m.translation_part() + m.col3(1).normalize() * length)).unwrap_or(head + Vec3::Y);
            let roll = match b.f32("arm_roll") {
                Some(r) if b.has("arm_roll") => r,
                _ => arm_mat.map_or(0.0, |m| mat3_vec_to_roll(&m.mat3(), tail - head)),
            };
            let flag = b.num("flag").unwrap_or(0.0) as i64;
            let mut bone = Bone::new(&b.string("name").unwrap_or_default(), head, tail);
            bone.parent = parent;
            bone.roll = if roll.is_finite() { roll } else { 0.0 };
            bone.use_connect = flag & 1 << 4 != 0;
            bone.use_inherit_rotation = flag & 1 << 9 == 0;
            bone.use_envelope_multiply = flag & 1 << 11 != 0;
            bone.use_deform = flag & 1 << 12 == 0;
            bone.use_local_location = flag & 1 << 22 == 0;
            bone.use_relative_parent = flag & 1 << 23 != 0;
            bone.inherit_scale = match b.num("inherit_scale_mode") {
                Some(m) => InheritScale::from_blender(m as i64),
                None if flag & 1 << 15 != 0 => InheritScale::NoneLegacy,
                None => InheritScale::Full,
            };
            let fl = |name: &str, d: f32| b.f32(name).filter(|x| x.is_finite()).unwrap_or(d);
            bone.envelope_distance = fl("dist", 0.25);
            bone.envelope_weight = fl("weight", 1.0);
            bone.head_radius = fl("rad_head", 0.1);
            bone.tail_radius = fl("rad_tail", 0.05);
            bone.bbone_segments = (b.num("segments").unwrap_or(1.0) as i64).clamp(1, 32) as u32;
            let i = arm.bones.len();
            arm.bones.push(bone);
            by_ptr.insert(ptr, i);
            stack.push((b.sub("childbase"), Some(i)));
        }
    }
    // Bone collections (4.0+): an array of BoneCollection pointers.
    let n = count(ar.num("collection_array_num").unwrap_or(0.0)).min(10_000);
    for p in f.ptr_array(ar.ptr("collection_array"), n) {
        let Some(c) = f.view_ptr(p, &["BoneCollection"]) else { continue };
        let name = c.string("name").unwrap_or_default();
        for m in f.list(c.sub("bones"), &["BoneCollectionMember"]) {
            if let Some(&bi) = by_ptr.get(&m.ptr("bone")) {
                if !arm.bones[bi].collections.contains(&name) {
                    arm.bones[bi].collections.push(name.clone());
                }
            }
        }
        arm.collections.push(name);
    }
    arm
}

/// Pose channels (`bPose.chanbase` of `bPoseChannel`).
pub(super) fn pose(f: &BlendFile<'_>, ob: &View<'_, '_>) -> Vec<PoseBone> {
    let Some(pose) = f.view_ptr(ob.ptr("pose"), &["bPose"]) else { return Vec::new() };
    f.list(pose.sub("chanbase"), &["bPoseChannel"])
        .iter()
        .map(|c| {
            let mut pb = PoseBone::new(&c.string("name").unwrap_or_default());
            let finite3 = |name: &str, d: Vec3| c.vec3(name).filter(|v| v.to_array().iter().all(|x| x.is_finite())).unwrap_or(d);
            pb.location = finite3("loc", Vec3::ZERO);
            pb.scale = c.vec3("size").or_else(|| c.vec3("scale")).filter(|v| v.to_array().iter().all(|x| x.is_finite())).unwrap_or(Vec3::ONE);
            pb.rotation_euler = finite3("eul", Vec3::ZERO);
            if let Some(q) = c.vec::<4>("quat").filter(|q| q.iter().all(|x| x.is_finite())) {
                pb.rotation_quaternion = Quat::new(q[0], q[1], q[2], q[3]);
            }
            let axis = finite3("rotAxis", Vec3::Y);
            pb.rotation_axis_angle = [c.f32("rotAngle").filter(|x| x.is_finite()).unwrap_or(0.0), axis.x, axis.y, axis.z];
            pb.rotation_mode = RotationMode::from_blender(c.num("rotmode").unwrap_or(0.0) as i64);
            let ik = c.num("ikflag").unwrap_or(0.0) as i64;
            pb.ik.lock = [ik & 1 != 0, ik & 2 != 0, ik & 4 != 0];
            pb.ik.use_limit = [ik & 8 != 0, ik & 16 != 0, ik & 32 != 0];
            if let Some(v) = c.vec::<3>("limitmin") {
                pb.ik.min = v;
            }
            if let Some(v) = c.vec::<3>("limitmax") {
                pb.ik.max = v;
            }
            if let Some(v) = c.vec::<3>("stiffness") {
                pb.ik.stiffness = v;
            }
            pb.ik.stretch = c.f32("ikstretch").filter(|x| x.is_finite()).unwrap_or(0.0);
            pb.constraints = constraints(f, c.sub("constraints"));
            pb
        })
        .collect()
}

fn object_name(f: &BlendFile<'_>, ptr: u64) -> String {
    f.view_ptr(ptr, &["Object"]).map(|o| o.id_name()).unwrap_or_default()
}

fn target(f: &BlendFile<'_>, d: &View<'_, '_>, tar: &str, sub: &str) -> Target {
    Target { object: object_name(f, d.ptr(tar)), subtarget: d.string(sub).unwrap_or_default(), head_tail: 0.0 }
}

fn track(v: f64) -> TrackAxis {
    TrackAxis::from_blender(v as i64)
}

fn axis3(v: f64) -> u8 {
    (v as i64).clamp(0, 2) as u8
}

fn euler_order(v: f64) -> Option<EulerOrder> {
    EulerOrder::from_blender(v as i64)
}

/// A constraint list (`ListBase` of `bConstraint`); types sex-3d does not
/// have are skipped.
pub(super) fn constraints(f: &BlendFile<'_>, lb: Option<View<'_, '_>>) -> Vec<Constraint> {
    let mut out = Vec::new();
    for c in f.list(lb, &["bConstraint"]) {
        let Some(d) = f.view_own(c.ptr("data")) else { continue };
        let Some(kind) = constraint_kind(f, &d) else { continue };
        let mut con = Constraint::new(kind);
        if let Some(n) = c.string("name").filter(|n| !n.is_empty()) {
            con.name = n;
        }
        con.influence = c.f32("enforce").filter(|x| x.is_finite()).unwrap_or(1.0).clamp(0.0, 1.0);
        con.mute = c.num("flag").unwrap_or(0.0) as i64 & 1 << 9 != 0;
        con.owner_space = Space::from_blender(c.num("ownspace").unwrap_or(0.0) as i64);
        con.target_space = Space::from_blender(c.num("tarspace").unwrap_or(0.0) as i64);
        con.space_object = object_name(f, c.ptr("space_object"));
        con.space_subtarget = c.string("space_subtarget").unwrap_or_default();
        let ht = c.f32("headtail").filter(|x| x.is_finite()).unwrap_or(0.0);
        if !matches!(con.kind, ConstraintKind::Armature { .. }) {
            if let Some(t) = con.target_mut() {
                t.head_tail = ht;
            }
        }
        out.push(con);
    }
    out
}

/// The type settings of one constraint, by its data struct's DNA name.
fn constraint_kind(f: &BlendFile<'_>, d: &View<'_, '_>) -> Option<ConstraintKind> {
    use ConstraintKind as K;
    let flag = |name: &str| d.num(name).unwrap_or(0.0) as i64;
    let num = |name: &str| d.num(name).unwrap_or(0.0);
    let fl = |name: &str, def: f32| d.f32(name).filter(|x| x.is_finite()).unwrap_or(def);
    let v3 = |name: &str| d.vec::<3>(name).filter(|v| v.iter().all(|x| x.is_finite())).unwrap_or([0.0; 3]);
    let t = || target(f, d, "tar", "subtarget");
    Some(match d.struct_name() {
        "bLocateLikeConstraint" => {
            let fg = flag("flag");
            K::CopyLocation { target: t(), use_xyz: [fg & 1 != 0, fg & 2 != 0, fg & 4 != 0], invert: [fg & 16 != 0, fg & 32 != 0, fg & 64 != 0], use_offset: fg & 128 != 0 }
        }
        "bRotateLikeConstraint" => {
            let fg = flag("flag");
            // ROTLIKE_MIX_REPLACE 0, OFFSET 1 (legacy), ADD 2, BEFORE 3, AFTER 4.
            let mix = match flag("mix_mode") {
                1 | 2 => RotationMix::Add,
                3 => RotationMix::Before,
                4 => RotationMix::After,
                _ if fg & 128 != 0 => RotationMix::Add,
                _ => RotationMix::Replace,
            };
            K::CopyRotation { target: t(), use_xyz: [fg & 1 != 0, fg & 4 != 0, fg & 16 != 0], invert: [fg & 2 != 0, fg & 8 != 0, fg & 32 != 0], mix, euler_order: euler_order(num("euler_order")) }
        }
        "bSizeLikeConstraint" => {
            let fg = flag("flag");
            K::CopyScale { target: t(), use_xyz: [fg & 1 != 0, fg & 2 != 0, fg & 4 != 0], power: fl("power", 1.0), use_make_uniform: fg & 32 != 0, use_offset: fg & 8 != 0, use_add: fg & 16 != 0 }
        }
        "bTransLikeConstraint" => K::CopyTransforms {
            target: t(),
            // TRANSLIKE_MIX_BEFORE 1, AFTER 2, BEFORE_SPLIT 3, AFTER_SPLIT 4,
            // BEFORE_FULL 5, AFTER_FULL 6.
            mix: match flag("mix_mode") {
                1 => TransformMix::Before,
                2 => TransformMix::After,
                3 => TransformMix::BeforeSplit,
                4 => TransformMix::AfterSplit,
                5 => TransformMix::BeforeFull,
                6 => TransformMix::AfterFull,
                _ => TransformMix::Replace,
            },
        },
        "bDistLimitConstraint" => K::LimitDistance {
            target: t(),
            distance: fl("dist", 0.0),
            mode: match flag("mode") {
                1 => LimitDistanceMode::Outside,
                2 => LimitDistanceMode::OnSurface,
                _ => LimitDistanceMode::Inside,
            },
        },
        "bLocLimitConstraint" | "bSizeLimitConstraint" => {
            let fg = flag("flag");
            let use_min = [fg & 1 != 0, fg & 4 != 0, fg & 16 != 0];
            let use_max = [fg & 2 != 0, fg & 8 != 0, fg & 32 != 0];
            let min = [fl("xmin", 0.0), fl("ymin", 0.0), fl("zmin", 0.0)];
            let max = [fl("xmax", 0.0), fl("ymax", 0.0), fl("zmax", 0.0)];
            if d.struct_name() == "bLocLimitConstraint" {
                K::LimitLocation { use_min, min, use_max, max }
            } else {
                K::LimitScale { use_min, min, use_max, max }
            }
        }
        "bRotLimitConstraint" => {
            let fg = flag("flag");
            K::LimitRotation {
                use_limit: [fg & 1 != 0, fg & 2 != 0, fg & 4 != 0],
                min: [fl("xmin", 0.0), fl("ymin", 0.0), fl("zmin", 0.0)],
                max: [fl("xmax", 0.0), fl("ymax", 0.0), fl("zmax", 0.0)],
                euler_order: euler_order(num("euler_order")),
            }
        }
        "bSameVolumeConstraint" => K::MaintainVolume {
            mode: match flag("mode") {
                1 => VolumeMode::Uniform,
                2 => VolumeMode::SingleAxis,
                _ => VolumeMode::Strict,
            },
            // The free axis is `flag` (SAMEVOL_X/Y/Z) in the DNA.
            free_axis: axis3(d.num_any(&["free_axis", "flag"]).unwrap_or(1.0)),
            volume: fl("volume", 1.0),
        },
        "bTransformConstraint" => {
            let chan = |v: i64| match v {
                1 => TransformChannel::Rotation,
                2 => TransformChannel::Scale,
                _ => TransformChannel::Location,
            };
            let (from, to) = (chan(flag("from")), chan(flag("to")));
            // 2.8+ keeps separate ranges per channel type.
            let range = |base: &str, ch: TransformChannel| -> [f32; 3] {
                let suffix = match ch {
                    TransformChannel::Location => "",
                    TransformChannel::Rotation => "_rot",
                    TransformChannel::Scale => "_scale",
                };
                let n = format!("{base}{suffix}");
                if d.has(&n) {
                    v3(&n)
                } else {
                    v3(base)
                }
            };
            let map = [axis3(d.num_at("map", 0).unwrap_or(0.0)), axis3(d.num_at("map", 1).unwrap_or(1.0)), axis3(d.num_at("map", 2).unwrap_or(2.0))];
            let mix = match to {
                TransformChannel::Location => {
                    if flag("mix_mode_loc") == 1 {
                        TransformationMix::Replace
                    } else {
                        TransformationMix::Add
                    }
                }
                TransformChannel::Rotation => match flag("mix_mode_rot") {
                    1 => TransformationMix::Replace,
                    2 => TransformationMix::Before,
                    3 => TransformationMix::After,
                    _ => TransformationMix::Add,
                },
                TransformChannel::Scale => {
                    if flag("mix_mode_scale") == 1 {
                        TransformationMix::Multiply
                    } else {
                        TransformationMix::Replace
                    }
                }
            };
            K::Transformation {
                target: t(),
                map_from: from,
                map_to: to,
                from_min: range("from_min", from),
                from_max: range("from_max", from),
                to_min: range("to_min", to),
                to_max: range("to_max", to),
                map,
                use_motion_extrapolate: flag("expo") != 0,
                mix,
                to_euler_order: euler_order(num("to_euler_order")),
            }
        }
        "bChildOfConstraint" => {
            let fg = flag("flag");
            let inverse = d.vec::<16>("invmat").filter(|m| m.iter().all(|x| x.is_finite())).map_or(Mat4::IDENTITY, Mat4::from_cols_array);
            K::ChildOf {
                target: t(),
                use_location: [fg & 1 != 0, fg & 2 != 0, fg & 4 != 0],
                use_rotation: [fg & 8 != 0, fg & 16 != 0, fg & 32 != 0],
                use_scale: [fg & 64 != 0, fg & 128 != 0, fg & 256 != 0],
                inverse,
            }
        }
        "bTrackToConstraint" => K::TrackTo { target: t(), track_axis: track(num("reserved1")), up_axis: axis3(num("reserved2")), use_target_z: flag("flags") & 1 != 0 },
        "bDampTrackConstraint" => K::DampedTrack { target: t(), track_axis: track(num("trackflag")) },
        "bLockTrackConstraint" => K::LockedTrack { target: t(), track_axis: track(num("trackflag")), lock_axis: axis3(num("lockflag")) },
        "bStretchToConstraint" => {
            let fg = flag("flag");
            K::StretchTo {
                target: t(),
                rest_length: fl("orglength", 0.0),
                bulge: fl("bulge", 1.0),
                use_bulge_min: fg & 1 != 0,
                bulge_min: fl("bulge_min", 0.0),
                use_bulge_max: fg & 2 != 0,
                bulge_max: fl("bulge_max", 0.0),
                bulge_smooth: fl("bulge_smooth", 0.0),
                volume: match flag("volmode") {
                    1 => StretchVolume::X,
                    2 => StretchVolume::Z,
                    3 => StretchVolume::None,
                    _ => StretchVolume::XZ,
                },
                // PLANE_X 0, SWING_Y 1, PLANE_Z 2.
                keep_axis: match flag("plane") {
                    1 => StretchPlane::SwingY,
                    2 => StretchPlane::Z,
                    _ => StretchPlane::X,
                },
            }
        }
        "bMinMaxConstraint" => K::Floor { target: t(), floor_location: track(num("minmaxflag")), offset: fl("offset", 0.0), use_rotation: flag("flag") & 4 != 0 },
        "bArmatureConstraint" => {
            let fg = flag("flag");
            let targets = f
                .list(d.sub("targets"), &["bConstraintTarget"])
                .iter()
                .map(|ct| (target(f, ct, "tar", "subtarget"), ct.f32("weight").filter(|x| x.is_finite()).unwrap_or(1.0)))
                .collect();
            K::Armature { targets, use_deform_preserve_volume: fg & 1 != 0, use_bone_envelopes: fg & 2 != 0 }
        }
        "bKinematicConstraint" => {
            let fg = flag("flag");
            let pole = object_name(f, d.ptr("poletar"));
            K::Ik {
                target: t(),
                pole_target: (!pole.is_empty()).then(|| Target { object: pole, subtarget: d.string("polesubtarget").unwrap_or_default(), head_tail: 0.0 }),
                pole_angle: fl("poleangle", 0.0),
                chain_count: (num("rootbone") as i64).clamp(0, 255) as u32,
                iterations: (num("iterations") as i64).clamp(1, 10_000) as u32,
                use_tail: fg & 1 != 0,
                use_stretch: fg & 16 != 0,
                weight: fl("weight", 1.0),
            }
        }
        _ => return None,
    })
}

/// Lattice data (`Lattice`: `pntsu/v/w`, `typeu/v/w`, `flag`, `def`).
pub(super) fn lattice(f: &BlendFile<'_>, lt: &View<'_, '_>) -> Lattice {
    let res = |n: &str| (lt.num(n).unwrap_or(2.0) as i64).clamp(1, 64) as u32;
    let mut l = Lattice::new(res("pntsu"), res("pntsv"), res("pntsw"));
    let ty = |n: &str| KeyInterpolation::from_blender(lt.num(n).unwrap_or(2.0) as i64);
    (l.interpolation_u, l.interpolation_v, l.interpolation_w) = (ty("typeu"), ty("typev"), ty("typew"));
    l.use_outside = lt.num("flag").unwrap_or(0.0) as i64 & 2 != 0;
    if let Some(pts) = f.view_array(lt.ptr("def"), &["BPoint"], l.point_count()) {
        let p: Option<Vec<Vec3>> = pts.iter().map(|b| b.vec::<4>("vec").filter(|v| v[..3].iter().all(|x| x.is_finite())).map(|v| Vec3::new(v[0], v[1], v[2]))).collect();
        if let Some(p) = p {
            l.points = p;
        }
    }
    l
}

/// The vertex group (name, inverted) of a deform modifier block.
pub(super) fn modifier_vertex_group(v: &View<'_, '_>) -> Option<(String, bool)> {
    let flag = |name: &str| v.num(name).unwrap_or(0.0) as i64;
    let (name, inv) = match v.struct_name() {
        "DisplaceModifierData" => ("defgrp_name", flag("flag") & 1 != 0),
        "SmoothModifierData" => ("defgrp_name", flag("flag") & 1 != 0),
        "SimpleDeformModifierData" => ("vgroup_name", flag("flag") & 1 != 0),
        "CastModifierData" => ("defgrp_name", flag("flag") & 1 != 0),
        "WaveModifierData" => ("defgrp_name", flag("flag") & 1 != 0),
        "ShrinkwrapModifierData" => ("vgroup_name", flag("shrinkOpts") & 1 << 6 != 0),
        "LaplacianSmoothModifierData" => ("defgrp_name", flag("flag") & 1 << 6 != 0),
        "CorrectiveSmoothModifierData" => ("defgrp_name", flag("flag") & 1 != 0),
        "ArmatureModifierData" => ("defgrp_name", flag("deformflag") & 16 != 0),
        "HookModifierData" => ("name", flag("flag") & 2 != 0),
        "LatticeModifierData" => ("name", flag("flag") & 1 != 0),
        "WarpModifierData" => ("defgrp_name", flag("flag") & 2 != 0),
        _ => return None,
    };
    let g = v.string(name).filter(|g| !g.is_empty());
    (g.is_some() || inv).then(|| (g.unwrap_or_default(), inv))
}

/// The rigging modifiers, by DNA struct name.
pub(super) fn modifier(f: &BlendFile<'_>, v: &View<'_, '_>) -> Option<Modifier> {
    let flag = |name: &str| v.num(name).unwrap_or(0.0) as i64;
    let fl = |name: &str, d: f32| v.f32(name).filter(|x| x.is_finite()).unwrap_or(d);
    let st = |name: &str| v.string(name).unwrap_or_default();
    let ob = |name: &str| object_name(f, v.ptr(name));
    Some(match v.struct_name() {
        "ArmatureModifierData" => {
            let df = flag("deformflag");
            Modifier::Armature {
                object: ob("object"),
                use_vertex_groups: df & 1 != 0,
                use_bone_envelopes: df & 2 != 0,
                use_deform_preserve_volume: df & 4 != 0,
                use_multi_modifier: flag("multi") != 0,
                vertex_group: None,
                invert_vertex_group: false,
            }
        }
        "HookModifierData" => {
            let n = count(v.num_any(&["indexar_num", "totindex"]).unwrap_or(0.0)).min(1 << 26);
            let vertex_indices = f
                .raw_array(v.ptr("indexar"), 4, n)
                .map(|d| (0..n).filter_map(|i| f.e.i32(d, i * 4)).filter(|&x| x >= 0).map(|x| x as u32).collect())
                .unwrap_or_default();
            Modifier::Hook {
                object: ob("object"),
                subtarget: st("subtarget"),
                falloff_type: Falloff::from_blender(flag("falloff_type"), false),
                falloff_radius: fl("falloff", 0.0),
                center: v.vec::<3>("cent").unwrap_or([0.0; 3]),
                matrix_inverse: v.vec::<16>("parentinv").filter(|m| m.iter().all(|x| x.is_finite())).map_or(Mat4::IDENTITY, Mat4::from_cols_array),
                strength: fl("force", 1.0),
                use_falloff_uniform: flag("flag") & 1 != 0,
                vertex_indices,
                vertex_group: None,
                invert_vertex_group: false,
            }
        }
        "LatticeModifierData" => Modifier::Lattice { object: ob("object"), strength: fl("strength", 1.0), vertex_group: None, invert_vertex_group: false },
        "WarpModifierData" => Modifier::Warp {
            object_from: ob("object_from"),
            object_to: ob("object_to"),
            strength: fl("strength", 1.0),
            falloff_type: Falloff::from_blender(flag("falloff_type"), false),
            falloff_radius: fl("falloff_radius", 1.0),
            use_volume_preserve: flag("flag") & 1 != 0,
            vertex_group: None,
            invert_vertex_group: false,
        },
        "WeightVGEditModifierData" => {
            let ef = flag("edit_flags");
            Modifier::VertexWeightEdit {
                vertex_group: st("defgrp_name"),
                default_weight: fl("default_weight", 0.0),
                use_add: ef & 1 << 3 != 0,
                add_threshold: fl("add_threshold", 0.01),
                use_remove: ef & 1 << 4 != 0,
                remove_threshold: fl("rem_threshold", 0.01),
                falloff_type: Falloff::from_blender(flag("falloff_type"), true),
                // MOD_WVG_EDIT_WEIGHTS_NORMALIZE 1, MOD_WVG_INVERT_FALLOFF 2.
                invert_falloff: ef & 1 << 1 != 0,
                normalize: ef & 1 != 0,
                mask_constant: fl("mask_constant", 1.0),
            }
        }
        "WeightVGMixModifierData" => {
            let mf = flag("flag");
            Modifier::VertexWeightMix {
                vertex_group_a: st("defgrp_name_a"),
                vertex_group_b: st("defgrp_name_b"),
                default_weight_a: fl("default_weight_a", 0.0),
                default_weight_b: fl("default_weight_b", 0.0),
                mix_mode: WeightMixMode::from_blender(flag("mix_mode")),
                mix_set: WeightMixSet::from_blender(flag("mix_set")),
                invert_vertex_group_a: mf & 4 != 0,
                invert_vertex_group_b: mf & 8 != 0,
                normalize: mf & 2 != 0,
                mask_constant: fl("mask_constant", 1.0),
            }
        }
        "WeightVGProximityModifierData" => {
            let pf = flag("proximity_flags");
            Modifier::VertexWeightProximity {
                vertex_group: st("defgrp_name"),
                target: ob("proximity_ob_target"),
                // MOD_WVG_PROXIMITY_OBJECT 1, GEOMETRY 2.
                proximity_mode: if flag("proximity_mode") == 2 { ProximityMode::Geometry } else { ProximityMode::Object },
                use_proximity_geometry: [pf & 1 != 0, pf & 2 != 0, pf & 4 != 0],
                min_dist: fl("min_dist", 0.0),
                max_dist: fl("max_dist", 1.0),
                falloff_type: Falloff::from_blender(flag("falloff_type"), true),
                invert_falloff: pf & 16 != 0,
                normalize: pf & 32 != 0,
                mask_constant: fl("mask_constant", 1.0),
            }
        }
        _ => return None,
    })
}

