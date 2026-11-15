// SPDX-License-Identifier: GPL-3.0-only
//! polyops commands for rigging data: vertex groups, shape keys, armatures,
//! poses, constraints, lattices and the options of the rigging modifiers.
//! The syntax is documented in the parent module.
use super::*;
use crate::armature::{Armature, Bone, InheritScale, PoseBone, RotationMode};
use crate::constraints::{
    Constraint, ConstraintKind, LimitDistanceMode, RotationMix, Space, StretchPlane, StretchVolume, Target, TrackAxis, TransformChannel, TransformMix, TransformationMix, VolumeMode,
};
use crate::lattice::Lattice;
use crate::math::{EulerOrder, Mat4};
use crate::mesh::{KeyInterpolation, ShapeKey};
use crate::modifiers::{Falloff, ProximityMode, WeightMixMode, WeightMixSet};
use crate::scene::{ArmatureWeights, ParentType};

/// Commands handled here.
pub(super) const COMMANDS: &[&str] = &[
    "armature_new",
    "add_armature",
    "bone",
    "bone_collection",
    "lattice_new",
    "lattice_set",
    "add_lattice",
    "vgroup",
    "vgroup_remove",
    "shape_key",
    "shape_key_remove",
    "shape_key_settings",
    "pose_bone",
    "constraint_add",
    "constraint_set",
    "constraint_remove",
    "constraint_target",
    "constraint_set_inverse",
    "parent_with_armature",
];

// ---------------------------------------------------------------------------
// Option helpers

/// Pick one of `names` (case-insensitive) for `key`.
fn pick<T: Copy>(cmd: &mut Cmd, key: &str, names: &[(&str, T)], into: &mut T) -> Res<()> {
    if let Some(t) = cmd.opt(key) {
        *into = names.iter().find(|(n, _)| n.eq_ignore_ascii_case(&t.text)).map(|(_, v)| *v).ok_or_else(|| {
            let all: Vec<String> = names.iter().map(|(n, _)| n.to_ascii_lowercase()).collect();
            format!("{key}: '{}' is not one of {}", t.text, all.join(", "))
        })?;
    }
    Ok(())
}

/// The polyops (lowercase) name of `v` in `names`.
fn lname<T: PartialEq + Copy>(names: &[(&str, T)], v: T) -> String {
    names.iter().find(|(_, x)| *x == v).map_or(String::new(), |(n, _)| n.to_ascii_lowercase())
}

fn of(cmd: &mut Cmd, key: &str, into: &mut f32) -> Res<()> {
    *into = cmd.f(key)?.unwrap_or(*into);
    Ok(())
}

fn ou(cmd: &mut Cmd, key: &str, into: &mut u32) -> Res<()> {
    *into = cmd.u(key)?.unwrap_or(*into);
    Ok(())
}

fn ob(cmd: &mut Cmd, key: &str, into: &mut bool) -> Res<()> {
    *into = cmd.b(key)?.unwrap_or(*into);
    Ok(())
}

fn os(cmd: &mut Cmd, key: &str, into: &mut String) {
    if let Some(t) = cmd.opt(key) {
        *into = t.text;
    }
}

/// `none` or a name.
fn oname(cmd: &mut Cmd, key: &str, into: &mut Option<String>) {
    if let Some(t) = cmd.opt(key) {
        *into = if t.is_word("none") || t.text.is_empty() { None } else { Some(t.text) };
    }
}

fn of3(cmd: &mut Cmd, key: &str, into: &mut [f32; 3]) -> Res<()> {
    if let Some(v) = cmd.v3(key)? {
        *into = v.to_array();
    }
    Ok(())
}

/// Three booleans: `1,0,1`.
fn ob3(cmd: &mut Cmd, key: &str, into: &mut [bool; 3]) -> Res<()> {
    if let Some(t) = cmd.opt(key) {
        let v: Vec<&str> = t.text.split(',').collect();
        if v.len() != 3 {
            return Err(format!("{key}= takes three booleans (1,0,1)"));
        }
        for (o, s) in into.iter_mut().zip(v) {
            *o = boolean(s).map_err(|e| format!("{key}: {e}"))?;
        }
    }
    Ok(())
}

fn fb3(v: &[bool; 3]) -> String {
    format!("{},{},{}", v[0] as u8, v[1] as u8, v[2] as u8)
}

fn f(v: f32) -> String {
    fmt_f(v)
}

fn b(v: bool) -> u8 {
    v as u8
}

fn qname(v: &Option<String>) -> String {
    v.as_deref().map_or_else(|| "none".to_string(), quote)
}

const AXIS3: [(&str, u8); 3] = [("x", 0), ("y", 1), ("z", 2)];
const FALLOFFS: [(&str, Falloff); 11] = [
    ("none", Falloff::None),
    ("curve", Falloff::Curve),
    ("sharp", Falloff::Sharp),
    ("smooth", Falloff::Smooth),
    ("root", Falloff::Root),
    ("linear", Falloff::Linear),
    ("constant", Falloff::Constant),
    ("sphere", Falloff::Sphere),
    ("inverse_square", Falloff::InverseSquare),
    ("random", Falloff::Random),
    ("step", Falloff::Step),
];
const PROXIMITY: [(&str, ProximityMode); 2] = [("object", ProximityMode::Object), ("geometry", ProximityMode::Geometry)];
const MIX_MODES: [(&str, WeightMixMode); 9] = [
    ("set", WeightMixMode::Set),
    ("add", WeightMixMode::Add),
    ("sub", WeightMixMode::Subtract),
    ("mul", WeightMixMode::Multiply),
    ("div", WeightMixMode::Divide),
    ("dif", WeightMixMode::Difference),
    ("avg", WeightMixMode::Average),
    ("min", WeightMixMode::Min),
    ("max", WeightMixMode::Max),
];
const MIX_SETS: [(&str, WeightMixSet); 5] = [("all", WeightMixSet::All), ("a", WeightMixSet::A), ("b", WeightMixSet::B), ("or", WeightMixSet::Or), ("and", WeightMixSet::And)];
const INTERP: [(&str, KeyInterpolation); 4] = KeyInterpolation::NAMES;
const INHERIT: [(&str, InheritScale); 6] = InheritScale::NAMES;
const SPACES: [(&str, Space); 5] = Space::NAMES;
const TRACKS: [(&str, TrackAxis); 6] = TrackAxis::NAMES;
const ROT_MIX: [(&str, RotationMix); 4] = [("replace", RotationMix::Replace), ("add", RotationMix::Add), ("before", RotationMix::Before), ("after", RotationMix::After)];
const TRANSFORM_MIX: [(&str, TransformMix); 7] = [
    ("replace", TransformMix::Replace),
    ("before_full", TransformMix::BeforeFull),
    ("before", TransformMix::Before),
    ("before_split", TransformMix::BeforeSplit),
    ("after_full", TransformMix::AfterFull),
    ("after", TransformMix::After),
    ("after_split", TransformMix::AfterSplit),
];
const LIMIT_DIST: [(&str, LimitDistanceMode); 3] = [("inside", LimitDistanceMode::Inside), ("outside", LimitDistanceMode::Outside), ("onsurface", LimitDistanceMode::OnSurface)];
const VOLUME: [(&str, VolumeMode); 3] = [("strict", VolumeMode::Strict), ("uniform", VolumeMode::Uniform), ("single_axis", VolumeMode::SingleAxis)];
const STRETCH_VOLUME: [(&str, StretchVolume); 4] = [("volume_xzx", StretchVolume::XZ), ("volume_x", StretchVolume::X), ("volume_z", StretchVolume::Z), ("no_volume", StretchVolume::None)];
const STRETCH_PLANE: [(&str, StretchPlane); 3] = [("plane_x", StretchPlane::X), ("swing_y", StretchPlane::SwingY), ("plane_z", StretchPlane::Z)];
const CHANNELS: [(&str, TransformChannel); 3] = [("location", TransformChannel::Location), ("rotation", TransformChannel::Rotation), ("scale", TransformChannel::Scale)];
const TRANSFORMATION_MIX: [(&str, TransformationMix); 5] = [
    ("replace", TransformationMix::Replace),
    ("add", TransformationMix::Add),
    ("before", TransformationMix::Before),
    ("after", TransformationMix::After),
    ("multiply", TransformationMix::Multiply),
];
const WEIGHTS: [(&str, ArmatureWeights); 4] = [("none", ArmatureWeights::None), ("empty", ArmatureWeights::Empty), ("envelope", ArmatureWeights::Envelope), ("automatic", ArmatureWeights::Automatic)];
const PARENT_TYPES: [(&str, ParentType); 3] = [("object", ParentType::Object), ("armature", ParentType::Armature), ("bone", ParentType::Bone)];

fn euler_opt(cmd: &mut Cmd, key: &str, into: &mut Option<EulerOrder>) -> Res<()> {
    if let Some(t) = cmd.opt(key) {
        *into = if t.text.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(EulerOrder::from_name(&t.text).ok_or_else(|| format!("{key}: '{}' is not auto or an Euler order (xyz, xzy, ...)", t.text))?)
        };
    }
    Ok(())
}

fn euler_name(o: Option<EulerOrder>) -> String {
    o.map_or("auto".into(), |o| o.name().to_ascii_lowercase())
}

// ---------------------------------------------------------------------------
// Modifiers

/// The vertex group fields of a modifier that has the common pair.
fn vg_mut(m: &mut Modifier) -> Option<(&mut Option<String>, &mut bool)> {
    match m {
        Modifier::Displace { vertex_group, invert_vertex_group, .. }
        | Modifier::Smooth { vertex_group, invert_vertex_group, .. }
        | Modifier::SimpleDeform { vertex_group, invert_vertex_group, .. }
        | Modifier::Cast { vertex_group, invert_vertex_group, .. }
        | Modifier::Wave { vertex_group, invert_vertex_group, .. }
        | Modifier::Shrinkwrap { vertex_group, invert_vertex_group, .. }
        | Modifier::LaplacianSmooth { vertex_group, invert_vertex_group, .. }
        | Modifier::CorrectiveSmooth { vertex_group, invert_vertex_group, .. }
        | Modifier::Armature { vertex_group, invert_vertex_group, .. }
        | Modifier::Hook { vertex_group, invert_vertex_group, .. }
        | Modifier::Lattice { vertex_group, invert_vertex_group, .. }
        | Modifier::Warp { vertex_group, invert_vertex_group, .. } => Some((vertex_group, invert_vertex_group)),
        _ => None,
    }
}

/// `vertex_group=` / `invert_vertex_group=` for every modifier that has
/// them, then the options of the rigging modifiers. True when `m` is a
/// rigging modifier (fully handled here).
pub(super) fn modifier_opts(cmd: &mut Cmd, m: &mut Modifier) -> Res<bool> {
    if let Some((g, inv)) = vg_mut(m) {
        oname(cmd, "vertex_group", g);
        ob(cmd, "invert_vertex_group", inv)?;
    }
    match m {
        Modifier::Armature { object, use_vertex_groups, use_bone_envelopes, use_deform_preserve_volume, use_multi_modifier, .. } => {
            os(cmd, "object", object);
            ob(cmd, "use_vertex_groups", use_vertex_groups)?;
            ob(cmd, "use_bone_envelopes", use_bone_envelopes)?;
            ob(cmd, "use_deform_preserve_volume", use_deform_preserve_volume)?;
            ob(cmd, "use_multi_modifier", use_multi_modifier)?;
        }
        Modifier::Hook { object, subtarget, falloff_type, falloff_radius, center, matrix_inverse, strength, use_falloff_uniform, vertex_indices, .. } => {
            os(cmd, "object", object);
            os(cmd, "subtarget", subtarget);
            pick(cmd, "falloff_type", &FALLOFFS, falloff_type)?;
            of(cmd, "falloff_radius", falloff_radius)?;
            of3(cmd, "center", center)?;
            if let Some(v) = cmd.list("matrix_inverse")? {
                *matrix_inverse = mat4_of(&v, "matrix_inverse")?;
            }
            of(cmd, "strength", strength)?;
            ob(cmd, "use_falloff_uniform", use_falloff_uniform)?;
            if let Some(t) = cmd.opt("vertex_indices") {
                *vertex_indices = uints(&t.text).map_err(|e| format!("vertex_indices: {e}"))?;
            }
        }
        Modifier::Lattice { object, strength, .. } => {
            os(cmd, "object", object);
            of(cmd, "strength", strength)?;
        }
        Modifier::Warp { object_from, object_to, strength, falloff_type, falloff_radius, use_volume_preserve, .. } => {
            os(cmd, "object_from", object_from);
            os(cmd, "object_to", object_to);
            of(cmd, "strength", strength)?;
            pick(cmd, "falloff_type", &FALLOFFS, falloff_type)?;
            of(cmd, "falloff_radius", falloff_radius)?;
            ob(cmd, "use_volume_preserve", use_volume_preserve)?;
        }
        Modifier::VertexWeightEdit { vertex_group, default_weight, use_add, add_threshold, use_remove, remove_threshold, falloff_type, invert_falloff, normalize, mask_constant } => {
            os(cmd, "vertex_group", vertex_group);
            of(cmd, "default_weight", default_weight)?;
            ob(cmd, "use_add", use_add)?;
            of(cmd, "add_threshold", add_threshold)?;
            ob(cmd, "use_remove", use_remove)?;
            of(cmd, "remove_threshold", remove_threshold)?;
            pick(cmd, "falloff_type", &FALLOFFS, falloff_type)?;
            ob(cmd, "invert_falloff", invert_falloff)?;
            ob(cmd, "normalize", normalize)?;
            of(cmd, "mask_constant", mask_constant)?;
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
            os(cmd, "vertex_group_a", vertex_group_a);
            os(cmd, "vertex_group_b", vertex_group_b);
            of(cmd, "default_weight_a", default_weight_a)?;
            of(cmd, "default_weight_b", default_weight_b)?;
            pick(cmd, "mix_mode", &MIX_MODES, mix_mode)?;
            pick(cmd, "mix_set", &MIX_SETS, mix_set)?;
            ob(cmd, "invert_vertex_group_a", invert_vertex_group_a)?;
            ob(cmd, "invert_vertex_group_b", invert_vertex_group_b)?;
            ob(cmd, "normalize", normalize)?;
            of(cmd, "mask_constant", mask_constant)?;
        }
        Modifier::VertexWeightProximity { vertex_group, target, proximity_mode, use_proximity_geometry, min_dist, max_dist, falloff_type, invert_falloff, normalize, mask_constant } => {
            os(cmd, "vertex_group", vertex_group);
            os(cmd, "target", target);
            pick(cmd, "proximity_mode", &PROXIMITY, proximity_mode)?;
            ob3(cmd, "use_proximity_geometry", use_proximity_geometry)?;
            of(cmd, "min_dist", min_dist)?;
            of(cmd, "max_dist", max_dist)?;
            pick(cmd, "falloff_type", &FALLOFFS, falloff_type)?;
            ob(cmd, "invert_falloff", invert_falloff)?;
            ob(cmd, "normalize", normalize)?;
            of(cmd, "mask_constant", mask_constant)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// 16 numbers, column by column.
fn mat4_of(v: &[f32], key: &str) -> Res<Mat4> {
    if v.len() != 16 {
        return Err(format!("{key}= takes 16 numbers (column-major)"));
    }
    let mut a = [0.0; 16];
    a.copy_from_slice(v);
    Ok(Mat4::from_cols_array(a))
}

/// The options [`modifier_opts`] reads, for writing: the vertex group pair
/// (when set) and everything of a rigging modifier.
pub(super) fn modifier_to_options(m: &Modifier) -> String {
    let mut o = String::new();
    let mut mc = m.clone();
    if let Some((g, inv)) = vg_mut(&mut mc) {
        if g.is_some() || *inv {
            let _ = write!(o, " vertex_group={} invert_vertex_group={}", qname(g), b(*inv));
        }
    }
    let _ = match m {
        Modifier::Armature { object, use_vertex_groups, use_bone_envelopes, use_deform_preserve_volume, use_multi_modifier, .. } => write!(
            o,
            " object={} use_vertex_groups={} use_bone_envelopes={} use_deform_preserve_volume={} use_multi_modifier={}",
            quote(object),
            b(*use_vertex_groups),
            b(*use_bone_envelopes),
            b(*use_deform_preserve_volume),
            b(*use_multi_modifier)
        ),
        Modifier::Hook { object, subtarget, falloff_type, falloff_radius, center, matrix_inverse, strength, use_falloff_uniform, vertex_indices, .. } => {
            let _ = write!(
                o,
                " object={} subtarget={} falloff_type={} falloff_radius={} center={} strength={} use_falloff_uniform={}",
                quote(object),
                quote(subtarget),
                lname(&FALLOFFS, *falloff_type),
                f(*falloff_radius),
                fmt_list(center),
                f(*strength),
                b(*use_falloff_uniform)
            );
            if *matrix_inverse != Mat4::IDENTITY {
                let _ = write!(o, " matrix_inverse={}", fmt_list(&matrix_inverse.to_cols_array()));
            }
            if !vertex_indices.is_empty() {
                let text = if vertex_indices.len() > BASE64_ABOVE { b64_u32(vertex_indices) } else { vertex_indices.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",") };
                let _ = write!(o, " vertex_indices={text}");
            }
            Ok(())
        }
        Modifier::Lattice { object, strength, .. } => write!(o, " object={} strength={}", quote(object), f(*strength)),
        Modifier::Warp { object_from, object_to, strength, falloff_type, falloff_radius, use_volume_preserve, .. } => write!(
            o,
            " object_from={} object_to={} strength={} falloff_type={} falloff_radius={} use_volume_preserve={}",
            quote(object_from),
            quote(object_to),
            f(*strength),
            lname(&FALLOFFS, *falloff_type),
            f(*falloff_radius),
            b(*use_volume_preserve)
        ),
        Modifier::VertexWeightEdit { vertex_group, default_weight, use_add, add_threshold, use_remove, remove_threshold, falloff_type, invert_falloff, normalize, mask_constant } => write!(
            o,
            " vertex_group={} default_weight={} use_add={} add_threshold={} use_remove={} remove_threshold={} falloff_type={} invert_falloff={} normalize={} mask_constant={}",
            quote(vertex_group),
            f(*default_weight),
            b(*use_add),
            f(*add_threshold),
            b(*use_remove),
            f(*remove_threshold),
            lname(&FALLOFFS, *falloff_type),
            b(*invert_falloff),
            b(*normalize),
            f(*mask_constant)
        ),
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
        } => write!(
            o,
            " vertex_group_a={} vertex_group_b={} default_weight_a={} default_weight_b={} mix_mode={} mix_set={} invert_vertex_group_a={} invert_vertex_group_b={} normalize={} mask_constant={}",
            quote(vertex_group_a),
            quote(vertex_group_b),
            f(*default_weight_a),
            f(*default_weight_b),
            lname(&MIX_MODES, *mix_mode),
            lname(&MIX_SETS, *mix_set),
            b(*invert_vertex_group_a),
            b(*invert_vertex_group_b),
            b(*normalize),
            f(*mask_constant)
        ),
        Modifier::VertexWeightProximity { vertex_group, target, proximity_mode, use_proximity_geometry, min_dist, max_dist, falloff_type, invert_falloff, normalize, mask_constant } => write!(
            o,
            " vertex_group={} target={} proximity_mode={} use_proximity_geometry={} min_dist={} max_dist={} falloff_type={} invert_falloff={} normalize={} mask_constant={}",
            quote(vertex_group),
            quote(target),
            lname(&PROXIMITY, *proximity_mode),
            fb3(use_proximity_geometry),
            f(*min_dist),
            f(*max_dist),
            lname(&FALLOFFS, *falloff_type),
            b(*invert_falloff),
            b(*normalize),
            f(*mask_constant)
        ),
        _ => Ok(()),
    };
    o
}

// ---------------------------------------------------------------------------
// Constraints

fn target_opts(cmd: &mut Cmd, t: &mut Target, prefix: &str) -> Res<()> {
    os(cmd, &format!("{prefix}target"), &mut t.object);
    os(cmd, &format!("{prefix}subtarget"), &mut t.subtarget);
    of(cmd, &format!("{prefix}head_tail"), &mut t.head_tail)
}

fn target_to_options(o: &mut String, t: &Target, prefix: &str) {
    let _ = write!(o, " {prefix}target={} {prefix}subtarget={}", quote(&t.object), quote(&t.subtarget));
    if t.head_tail != 0.0 {
        let _ = write!(o, " {prefix}head_tail={}", f(t.head_tail));
    }
}

fn constraint_opts(cmd: &mut Cmd, c: &mut Constraint) -> Res<()> {
    use ConstraintKind as K;
    if let Some(t) = cmd.opt("name") {
        c.name = t.text;
    }
    of(cmd, "influence", &mut c.influence)?;
    c.influence = c.influence.clamp(0.0, 1.0);
    ob(cmd, "mute", &mut c.mute)?;
    pick(cmd, "owner_space", &SPACES, &mut c.owner_space)?;
    pick(cmd, "target_space", &SPACES, &mut c.target_space)?;
    os(cmd, "space_object", &mut c.space_object);
    os(cmd, "space_subtarget", &mut c.space_subtarget);
    if !matches!(c.kind, K::Armature { .. }) {
        if let Some(t) = c.target_mut() {
            target_opts(cmd, t, "")?;
        }
    }
    match &mut c.kind {
        K::CopyLocation { use_xyz, invert, use_offset, .. } => {
            ob3(cmd, "use_xyz", use_xyz)?;
            ob3(cmd, "invert", invert)?;
            ob(cmd, "use_offset", use_offset)?;
        }
        K::CopyRotation { use_xyz, invert, mix, euler_order, .. } => {
            ob3(cmd, "use_xyz", use_xyz)?;
            ob3(cmd, "invert", invert)?;
            pick(cmd, "mix_mode", &ROT_MIX, mix)?;
            euler_opt(cmd, "euler_order", euler_order)?;
        }
        K::CopyScale { use_xyz, power, use_make_uniform, use_offset, use_add, .. } => {
            ob3(cmd, "use_xyz", use_xyz)?;
            of(cmd, "power", power)?;
            ob(cmd, "use_make_uniform", use_make_uniform)?;
            ob(cmd, "use_offset", use_offset)?;
            ob(cmd, "use_add", use_add)?;
        }
        K::CopyTransforms { mix, .. } => pick(cmd, "mix_mode", &TRANSFORM_MIX, mix)?,
        K::LimitDistance { distance, mode, .. } => {
            of(cmd, "distance", distance)?;
            pick(cmd, "limit_mode", &LIMIT_DIST, mode)?;
        }
        K::LimitLocation { use_min, min, use_max, max } | K::LimitScale { use_min, min, use_max, max } => {
            ob3(cmd, "use_min", use_min)?;
            of3(cmd, "min", min)?;
            ob3(cmd, "use_max", use_max)?;
            of3(cmd, "max", max)?;
        }
        K::LimitRotation { use_limit, min, max, euler_order } => {
            ob3(cmd, "use_limit", use_limit)?;
            of3(cmd, "min", min)?;
            of3(cmd, "max", max)?;
            euler_opt(cmd, "euler_order", euler_order)?;
        }
        K::MaintainVolume { mode, free_axis, volume } => {
            pick(cmd, "mode", &VOLUME, mode)?;
            pick(cmd, "free_axis", &AXIS3, free_axis)?;
            of(cmd, "volume", volume)?;
        }
        K::Transformation { map_from, map_to, from_min, from_max, to_min, to_max, map, use_motion_extrapolate, mix, to_euler_order, .. } => {
            pick(cmd, "map_from", &CHANNELS, map_from)?;
            pick(cmd, "map_to", &CHANNELS, map_to)?;
            of3(cmd, "from_min", from_min)?;
            of3(cmd, "from_max", from_max)?;
            of3(cmd, "to_min", to_min)?;
            of3(cmd, "to_max", to_max)?;
            if let Some(t) = cmd.opt("map") {
                let v: Vec<&str> = t.text.split(',').collect();
                if v.len() != 3 {
                    return Err("map= takes three axes (x,y,z)".into());
                }
                for (o, s) in map.iter_mut().zip(v) {
                    *o = AXIS3.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, a)| *a).ok_or_else(|| format!("map: '{s}' is not x, y or z"))?;
                }
            }
            ob(cmd, "use_motion_extrapolate", use_motion_extrapolate)?;
            pick(cmd, "mix_mode", &TRANSFORMATION_MIX, mix)?;
            euler_opt(cmd, "to_euler_order", to_euler_order)?;
        }
        K::ChildOf { use_location, use_rotation, use_scale, inverse, .. } => {
            ob3(cmd, "use_location", use_location)?;
            ob3(cmd, "use_rotation", use_rotation)?;
            ob3(cmd, "use_scale", use_scale)?;
            if let Some(v) = cmd.list("inverse_matrix")? {
                *inverse = mat4_of(&v, "inverse_matrix")?;
            }
        }
        K::TrackTo { track_axis, up_axis, use_target_z, .. } => {
            pick(cmd, "track_axis", &TRACKS, track_axis)?;
            pick(cmd, "up_axis", &AXIS3, up_axis)?;
            ob(cmd, "use_target_z", use_target_z)?;
        }
        K::DampedTrack { track_axis, .. } => pick(cmd, "track_axis", &TRACKS, track_axis)?,
        K::LockedTrack { track_axis, lock_axis, .. } => {
            pick(cmd, "track_axis", &TRACKS, track_axis)?;
            pick(cmd, "lock_axis", &AXIS3, lock_axis)?;
        }
        K::StretchTo { rest_length, bulge, use_bulge_min, bulge_min, use_bulge_max, bulge_max, bulge_smooth, volume, keep_axis, .. } => {
            of(cmd, "rest_length", rest_length)?;
            of(cmd, "bulge", bulge)?;
            ob(cmd, "use_bulge_min", use_bulge_min)?;
            of(cmd, "bulge_min", bulge_min)?;
            ob(cmd, "use_bulge_max", use_bulge_max)?;
            of(cmd, "bulge_max", bulge_max)?;
            of(cmd, "bulge_smooth", bulge_smooth)?;
            pick(cmd, "volume", &STRETCH_VOLUME, volume)?;
            pick(cmd, "keep_axis", &STRETCH_PLANE, keep_axis)?;
        }
        K::Floor { floor_location, offset, use_rotation, .. } => {
            pick(cmd, "floor_location", &TRACKS, floor_location)?;
            of(cmd, "offset", offset)?;
            ob(cmd, "use_rotation", use_rotation)?;
        }
        K::Armature { use_deform_preserve_volume, use_bone_envelopes, .. } => {
            ob(cmd, "use_deform_preserve_volume", use_deform_preserve_volume)?;
            ob(cmd, "use_bone_envelopes", use_bone_envelopes)?;
        }
        K::Ik { pole_target, pole_angle, chain_count, iterations, use_tail, use_stretch, weight, .. } => {
            if let Some(t) = cmd.opt("pole_target") {
                if t.is_word("none") || t.text.is_empty() {
                    *pole_target = None;
                } else {
                    pole_target.get_or_insert_with(Target::default).object = t.text;
                }
            }
            if let Some(t) = cmd.opt("pole_subtarget") {
                pole_target.get_or_insert_with(Target::default).subtarget = t.text;
            }
            of(cmd, "pole_angle", pole_angle)?;
            ou(cmd, "chain_count", chain_count)?;
            ou(cmd, "iterations", iterations)?;
            ob(cmd, "use_tail", use_tail)?;
            ob(cmd, "use_stretch", use_stretch)?;
            of(cmd, "weight", weight)?;
        }
    }
    Ok(())
}

fn constraint_to_options(c: &Constraint) -> String {
    use ConstraintKind as K;
    let mut o = format!(
        " name={} influence={} mute={} owner_space={} target_space={}",
        quote(&c.name),
        f(c.influence),
        b(c.mute),
        lname(&SPACES, c.owner_space),
        lname(&SPACES, c.target_space)
    );
    if !c.space_object.is_empty() || !c.space_subtarget.is_empty() {
        let _ = write!(o, " space_object={} space_subtarget={}", quote(&c.space_object), quote(&c.space_subtarget));
    }
    if !matches!(c.kind, K::Armature { .. }) {
        if let Some(t) = c.target() {
            target_to_options(&mut o, t, "");
        }
    }
    let _ = match &c.kind {
        K::CopyLocation { use_xyz, invert, use_offset, .. } => write!(o, " use_xyz={} invert={} use_offset={}", fb3(use_xyz), fb3(invert), b(*use_offset)),
        K::CopyRotation { use_xyz, invert, mix, euler_order, .. } => {
            write!(o, " use_xyz={} invert={} mix_mode={} euler_order={}", fb3(use_xyz), fb3(invert), lname(&ROT_MIX, *mix), euler_name(*euler_order))
        }
        K::CopyScale { use_xyz, power, use_make_uniform, use_offset, use_add, .. } => {
            write!(o, " use_xyz={} power={} use_make_uniform={} use_offset={} use_add={}", fb3(use_xyz), f(*power), b(*use_make_uniform), b(*use_offset), b(*use_add))
        }
        K::CopyTransforms { mix, .. } => write!(o, " mix_mode={}", lname(&TRANSFORM_MIX, *mix)),
        K::LimitDistance { distance, mode, .. } => write!(o, " distance={} limit_mode={}", f(*distance), lname(&LIMIT_DIST, *mode)),
        K::LimitLocation { use_min, min, use_max, max } | K::LimitScale { use_min, min, use_max, max } => {
            write!(o, " use_min={} min={} use_max={} max={}", fb3(use_min), fmt_list(min), fb3(use_max), fmt_list(max))
        }
        K::LimitRotation { use_limit, min, max, euler_order } => write!(o, " use_limit={} min={} max={} euler_order={}", fb3(use_limit), fmt_list(min), fmt_list(max), euler_name(*euler_order)),
        K::MaintainVolume { mode, free_axis, volume } => write!(o, " mode={} free_axis={} volume={}", lname(&VOLUME, *mode), lname(&AXIS3, *free_axis), f(*volume)),
        K::Transformation { map_from, map_to, from_min, from_max, to_min, to_max, map, use_motion_extrapolate, mix, to_euler_order, .. } => write!(
            o,
            " map_from={} map_to={} from_min={} from_max={} to_min={} to_max={} map={} use_motion_extrapolate={} mix_mode={} to_euler_order={}",
            lname(&CHANNELS, *map_from),
            lname(&CHANNELS, *map_to),
            fmt_list(from_min),
            fmt_list(from_max),
            fmt_list(to_min),
            fmt_list(to_max),
            map.iter().map(|a| lname(&AXIS3, (*a).min(2))).collect::<Vec<_>>().join(","),
            b(*use_motion_extrapolate),
            lname(&TRANSFORMATION_MIX, *mix),
            euler_name(*to_euler_order)
        ),
        K::ChildOf { use_location, use_rotation, use_scale, inverse, .. } => {
            let _ = write!(o, " use_location={} use_rotation={} use_scale={}", fb3(use_location), fb3(use_rotation), fb3(use_scale));
            if *inverse != Mat4::IDENTITY {
                let _ = write!(o, " inverse_matrix={}", fmt_list(&inverse.to_cols_array()));
            }
            Ok(())
        }
        K::TrackTo { track_axis, up_axis, use_target_z, .. } => write!(o, " track_axis={} up_axis={} use_target_z={}", lname(&TRACKS, *track_axis), lname(&AXIS3, *up_axis), b(*use_target_z)),
        K::DampedTrack { track_axis, .. } => write!(o, " track_axis={}", lname(&TRACKS, *track_axis)),
        K::LockedTrack { track_axis, lock_axis, .. } => write!(o, " track_axis={} lock_axis={}", lname(&TRACKS, *track_axis), lname(&AXIS3, *lock_axis)),
        K::StretchTo { rest_length, bulge, use_bulge_min, bulge_min, use_bulge_max, bulge_max, bulge_smooth, volume, keep_axis, .. } => write!(
            o,
            " rest_length={} bulge={} use_bulge_min={} bulge_min={} use_bulge_max={} bulge_max={} bulge_smooth={} volume={} keep_axis={}",
            f(*rest_length),
            f(*bulge),
            b(*use_bulge_min),
            f(*bulge_min),
            b(*use_bulge_max),
            f(*bulge_max),
            f(*bulge_smooth),
            lname(&STRETCH_VOLUME, *volume),
            lname(&STRETCH_PLANE, *keep_axis)
        ),
        K::Floor { floor_location, offset, use_rotation, .. } => write!(o, " floor_location={} offset={} use_rotation={}", lname(&TRACKS, *floor_location), f(*offset), b(*use_rotation)),
        K::Armature { use_deform_preserve_volume, use_bone_envelopes, .. } => write!(o, " use_deform_preserve_volume={} use_bone_envelopes={}", b(*use_deform_preserve_volume), b(*use_bone_envelopes)),
        K::Ik { pole_target, pole_angle, chain_count, iterations, use_tail, use_stretch, weight, .. } => {
            if let Some(p) = pole_target {
                let _ = write!(o, " pole_target={} pole_subtarget={}", quote(&p.object), quote(&p.subtarget));
            }
            write!(o, " pole_angle={} chain_count={chain_count} iterations={iterations} use_tail={} use_stretch={} weight={}", f(*pole_angle), b(*use_tail), b(*use_stretch), f(*weight))
        }
    };
    o
}

// ---------------------------------------------------------------------------
// Commands

fn pose_bone_mut<'a>(pose: &'a mut Vec<PoseBone>, name: &str) -> &'a mut PoseBone {
    match pose.iter().position(|p| p.name == name) {
        Some(i) => &mut pose[i],
        None => {
            pose.push(PoseBone::new(name));
            pose.last_mut().expect("pushed")
        }
    }
}

impl Session {
    fn armature_data(&self, scene: &Scene, t: &Token) -> Res<usize> {
        match self.handle(t)? {
            Some(Handle::Armature(i)) => return Ok(*i),
            Some(Handle::Object(_)) | None => {}
            Some(_) => return Err(format!("{} is not an armature", t.text)),
        }
        if let Ok(o) = self.object(scene, t) {
            return match scene.objects[o].data {
                ObjectData::Armature(a) => Ok(a),
                _ => Err(format!("object '{}' is not an armature", scene.objects[o].name)),
            };
        }
        scene.armature_names.iter().position(|n| *n == t.text).ok_or_else(|| format!("no armature named '{}'", t.text))
    }

    fn lattice_data(&self, scene: &Scene, t: &Token) -> Res<usize> {
        match self.handle(t)? {
            Some(Handle::Lattice(i)) => return Ok(*i),
            Some(Handle::Object(_)) | None => {}
            Some(_) => return Err(format!("{} is not a lattice", t.text)),
        }
        if let Ok(o) = self.object(scene, t) {
            return match scene.objects[o].data {
                ObjectData::Lattice(l) => Ok(l),
                _ => Err(format!("object '{}' is not a lattice", scene.objects[o].name)),
            };
        }
        scene.lattice_names.iter().position(|n| *n == t.text).ok_or_else(|| format!("no lattice named '{}'", t.text))
    }

    /// The constraint list addressed by `OBJ` and `bone=`.
    fn constraints_of<'a>(&self, scene: &'a mut Scene, cmd: &mut Cmd) -> Res<&'a mut Vec<Constraint>> {
        let o = self.object(scene, cmd.arg(0, "an object")?)?;
        Ok(match cmd.opt("bone") {
            Some(bn) => {
                if !matches!(scene.objects[o].data, ObjectData::Armature(_)) {
                    return Err(format!("'{}' is not an armature (bone= is for bone constraints)", scene.objects[o].name));
                }
                &mut pose_bone_mut(&mut scene.objects[o].pose, &bn.text).constraints
            }
            None => &mut scene.objects[o].constraints,
        })
    }

    fn constraint_index(list: &[Constraint], cmd: &Cmd) -> Res<usize> {
        let i = uint(&cmd.arg(1, "a constraint index")?.text)? as usize;
        if i >= list.len() {
            return Err(format!("constraint {i} does not exist (there are {})", list.len()));
        }
        Ok(i)
    }

    /// Run a rigging command (`name` is in [`COMMANDS`]).
    pub(super) fn rig_command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        match cmd.name.as_str() {
            "armature_new" => {
                cmd.max_args(1)?;
                let name = cmd.arg(0, "an armature name")?.text.clone();
                let i = scene.add_armature(&name, Armature::default());
                self.bind(cmd, Handle::Armature(i))?;
                Ok(Some(quote(&scene.armature_names[i])))
            }
            "add_armature" | "add_lattice" => {
                cmd.max_args(1)?;
                let lat = cmd.name == "add_lattice";
                let name = given_name(cmd, true).unwrap_or_else(|| if lat { "Lattice" } else { "Armature" }.into());
                let data = match cmd.opt("data") {
                    Some(t) if lat => ObjectData::Lattice(self.lattice_data(scene, &t)?),
                    Some(t) => ObjectData::Armature(self.armature_data(scene, &t)?),
                    None if lat => ObjectData::Lattice(scene.add_lattice(&name, Lattice::default())),
                    None => ObjectData::Armature(scene.add_armature(&name, Armature::default())),
                };
                let i = scene.add_object(Object::new(&name, data));
                self.created(scene, cmd, i).map(Some)
            }
            "bone" => {
                cmd.max_args(2)?;
                let a = self.armature_data(scene, cmd.arg(0, "an armature")?)?;
                let name = cmd.arg(1, "a bone name")?.text.clone();
                let parent = match cmd.opt("parent") {
                    Some(t) if t.is_word("none") => Some(None),
                    Some(t) => Some(Some(scene.armatures[a].find(&t.text).ok_or_else(|| format!("no bone named '{}'", t.text))?)),
                    None => None,
                };
                let arm = &mut scene.armatures[a];
                let bi = match arm.find(&name) {
                    Some(i) => i,
                    None => {
                        if name.is_empty() {
                            return Err("a bone name cannot be empty".into());
                        }
                        arm.add_bone(Bone::new(&name, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)))
                    }
                };
                if let Some(p) = parent {
                    if p.is_some_and(|p| p == bi || arm.is_ancestor(bi, p)) {
                        return Err("bone parenting would make a cycle".into());
                    }
                    arm.bones[bi].parent = p;
                }
                let bone = &mut arm.bones[bi];
                if let Some(v) = cmd.v3("head")? {
                    bone.head = v;
                }
                if let Some(v) = cmd.v3("tail")? {
                    bone.tail = v;
                }
                of(cmd, "roll", &mut bone.roll)?;
                ob(cmd, "use_connect", &mut bone.use_connect)?;
                ob(cmd, "use_deform", &mut bone.use_deform)?;
                ob(cmd, "use_inherit_rotation", &mut bone.use_inherit_rotation)?;
                pick(cmd, "inherit_scale", &INHERIT, &mut bone.inherit_scale)?;
                ob(cmd, "use_local_location", &mut bone.use_local_location)?;
                ob(cmd, "use_relative_parent", &mut bone.use_relative_parent)?;
                of(cmd, "envelope_distance", &mut bone.envelope_distance)?;
                of(cmd, "envelope_weight", &mut bone.envelope_weight)?;
                of(cmd, "head_radius", &mut bone.head_radius)?;
                of(cmd, "tail_radius", &mut bone.tail_radius)?;
                ob(cmd, "use_envelope_multiply", &mut bone.use_envelope_multiply)?;
                ou(cmd, "bbone_segments", &mut bone.bbone_segments)?;
                bone.bbone_segments = bone.bbone_segments.clamp(1, 32);
                Ok(Some(quote(&bone.name)))
            }
            "bone_collection" => {
                let a = self.armature_data(scene, cmd.arg(0, "an armature")?)?;
                let coll = cmd.arg(1, "a bone collection name")?.text.clone();
                let arm = &mut scene.armatures[a];
                if !arm.collections.contains(&coll) {
                    arm.collections.push(coll.clone());
                }
                for t in &cmd.pos[2..] {
                    let bi = arm.find(&t.text).ok_or_else(|| format!("no bone named '{}'", t.text))?;
                    if !arm.bones[bi].collections.contains(&coll) {
                        arm.bones[bi].collections.push(coll.clone());
                    }
                }
                Ok(None)
            }
            "lattice_new" | "lattice_set" => {
                cmd.max_args(1)?;
                let li = if cmd.name == "lattice_new" {
                    let name = cmd.arg(0, "a lattice name")?.text.clone();
                    let li = scene.add_lattice(&name, Lattice::default());
                    self.bind(cmd, Handle::Lattice(li))?;
                    li
                } else {
                    self.lattice_data(scene, cmd.arg(0, "a lattice")?)?
                };
                let l = &mut scene.lattices[li];
                if let Some(p) = cmd.list("points")? {
                    let ok = p.len() == 3 && p.iter().all(|x| x.fract() == 0.0 && (1.0..=64.0).contains(x));
                    if !ok {
                        return Err("points= takes three whole numbers from 1 to 64".into());
                    }
                    l.resize(p[0] as u32, p[1] as u32, p[2] as u32);
                }
                if let Some(t) = cmd.opt("interpolation") {
                    let v: Vec<&str> = t.text.split(',').collect();
                    let one = |s: &str| INTERP.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)).map(|(_, i)| *i).ok_or_else(|| format!("interpolation: '{s}' is not linear, cardinal, catmull_rom or bspline"));
                    match v.as_slice() {
                        [a] => {
                            let i = one(a)?;
                            (l.interpolation_u, l.interpolation_v, l.interpolation_w) = (i, i, i);
                        }
                        [a, b, c] => (l.interpolation_u, l.interpolation_v, l.interpolation_w) = (one(a)?, one(b)?, one(c)?),
                        _ => return Err("interpolation= takes one type or three (u,v,w)".into()),
                    }
                }
                ob(cmd, "use_outside", &mut l.use_outside)?;
                if let Some(v) = cmd.list("verts")? {
                    if v.len() != l.point_count() * 3 {
                        return Err(format!("verts= holds {} numbers for {} lattice points", v.len(), l.point_count()));
                    }
                    l.points = v.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
                }
                Ok(Some(quote(&scene.lattice_names[li])))
            }
            "vgroup" => {
                cmd.max_args(2)?;
                let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
                let name = cmd.arg(1, "a vertex group name")?.text.clone();
                if name.is_empty() {
                    return Err("a vertex group name cannot be empty".into());
                }
                let m = &mut scene.meshes[mi];
                let g = m.ensure_vertex_group(&name);
                let idx = cmd.opt("indices").map(|t| uints(&t.text).map_err(|e| format!("indices: {e}"))).transpose()?;
                let w = cmd.list("weights")?;
                let n = m.positions.len();
                match (idx, w) {
                    (None, None) => {}
                    (idx, w) => {
                        let idx: Vec<u32> = idx.unwrap_or_else(|| (0..n as u32).collect());
                        let w = w.unwrap_or_else(|| vec![1.0]);
                        if w.len() != 1 && w.len() != idx.len() {
                            return Err(format!("weights= has {} numbers for {} vertices (give one, or one each)", w.len(), idx.len()));
                        }
                        for (k, &v) in idx.iter().enumerate() {
                            if v as usize >= n {
                                return Err(format!("vertex {v} does not exist (the mesh has {n})"));
                            }
                            m.set_weight(v as usize, g, if w.len() == 1 { w[0] } else { w[k] });
                        }
                    }
                }
                if let Some(t) = cmd.opt("remove") {
                    for v in uints(&t.text).map_err(|e| format!("remove: {e}"))? {
                        m.remove_weight(v as usize, g);
                    }
                }
                Ok(Some(g.to_string()))
            }
            "vgroup_remove" => {
                cmd.max_args(2)?;
                let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
                let name = cmd.arg(1, "a vertex group name")?.text.clone();
                let m = &mut scene.meshes[mi];
                let g = m.vertex_group_index(&name).ok_or_else(|| format!("no vertex group named '{name}'"))?;
                m.remove_vertex_group(g);
                Ok(None)
            }
            "shape_key" => {
                cmd.max_args(2)?;
                let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
                let name = cmd.arg(1, "a shape key name")?.text.clone();
                let from_mix = cmd.b("from_mix")?.unwrap_or(false);
                let m = &mut scene.meshes[mi];
                let k = match m.shape_key_index(&name) {
                    Some(k) => k,
                    None => m.add_shape_key(&name, from_mix),
                };
                if let Some(v) = cmd.list("verts")? {
                    if v.len() != m.positions.len() * 3 {
                        return Err(format!("verts= holds {} numbers for {} vertices", v.len(), m.positions.len()));
                    }
                    m.shape_keys[k].positions = v.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
                }
                if let Some(t) = cmd.opt("relative_key") {
                    m.shape_keys[k].relative_key = m.shape_key_index(&t.text).ok_or_else(|| format!("no shape key named '{}'", t.text))?;
                }
                let key: &mut ShapeKey = &mut m.shape_keys[k];
                of(cmd, "value", &mut key.value)?;
                of(cmd, "slider_min", &mut key.slider_min)?;
                of(cmd, "slider_max", &mut key.slider_max)?;
                oname(cmd, "vertex_group", &mut key.vertex_group);
                ob(cmd, "mute", &mut key.mute)?;
                pick(cmd, "interpolation", &INTERP, &mut key.interpolation)?;
                of(cmd, "frame", &mut key.frame)?;
                Ok(Some(quote(&key.name)))
            }
            "shape_key_remove" => {
                cmd.max_args(2)?;
                let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
                let name = cmd.arg(1, "a shape key name")?.text.clone();
                let m = &mut scene.meshes[mi];
                let k = m.shape_key_index(&name).ok_or_else(|| format!("no shape key named '{name}'"))?;
                m.shape_keys.remove(k);
                for key in &mut m.shape_keys {
                    if key.relative_key == k {
                        key.relative_key = 0;
                    } else if key.relative_key > k {
                        key.relative_key -= 1;
                    }
                }
                Ok(None)
            }
            "shape_key_settings" => {
                cmd.max_args(1)?;
                let mi = self.mesh(scene, cmd.arg(0, "an object or mesh")?)?;
                let s = &mut scene.meshes[mi].shape_key_settings;
                ob(cmd, "use_relative", &mut s.use_relative)?;
                of(cmd, "eval_time", &mut s.eval_time)?;
                Ok(None)
            }
            "pose_bone" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an armature object")?)?;
                if !matches!(scene.objects[o].data, ObjectData::Armature(_)) {
                    return Err(format!("'{}' is not an armature", scene.objects[o].name));
                }
                let name = cmd.arg(1, "a bone name")?.text.clone();
                let pb = pose_bone_mut(&mut scene.objects[o].pose, &name);
                if let Some(v) = cmd.v3("location")? {
                    pb.location = v;
                }
                if let Some(t) = cmd.opt("rotation_mode") {
                    pb.rotation_mode = RotationMode::from_name(&t.text).ok_or_else(|| format!("rotation_mode: '{}' is not quaternion, axis_angle or an Euler order", t.text))?;
                }
                if let Some(q) = cmd.list("quaternion")? {
                    pb.rotation_quaternion = quat_of(&q)?;
                }
                if let Some(v) = cmd.v3("euler")? {
                    pb.rotation_euler = v;
                }
                if let Some(v) = cmd.list("axis_angle")? {
                    if v.len() != 4 {
                        return Err("axis_angle= takes angle,x,y,z".into());
                    }
                    pb.rotation_axis_angle = [v[0], v[1], v[2], v[3]];
                }
                if let Some(s) = cmd.list("scale")? {
                    pb.scale = scale_of(&s)?;
                }
                let ik = &mut pb.ik;
                ob3(cmd, "ik_lock", &mut ik.lock)?;
                of3(cmd, "ik_stiffness", &mut ik.stiffness)?;
                of(cmd, "ik_stretch", &mut ik.stretch)?;
                ob3(cmd, "ik_limit", &mut ik.use_limit)?;
                of3(cmd, "ik_min", &mut ik.min)?;
                of3(cmd, "ik_max", &mut ik.max)?;
                Ok(None)
            }
            "constraint_add" => {
                cmd.max_args(2)?;
                let ty = cmd.arg(1, "a constraint type")?.text.clone();
                let bone = cmd.opts.iter().any(|o| o.0 == "bone");
                let mut c = Constraint::with_defaults(&ty, bone).ok_or_else(|| {
                    format!(
                        "unknown constraint type '{ty}' (copy_location, copy_rotation, copy_scale, copy_transforms, limit_distance, limit_location, limit_rotation, limit_scale, \
                         maintain_volume, transformation, child_of, track_to, damped_track, locked_track, stretch_to, floor, armature, ik)"
                    )
                })?;
                constraint_opts(cmd, &mut c)?;
                let list = self.constraints_of(scene, cmd)?;
                list.push(c);
                Ok(Some((list.len() - 1).to_string()))
            }
            "constraint_set" | "constraint_remove" => {
                cmd.max_args(2)?;
                let remove = cmd.name == "constraint_remove";
                // Options are read before the list is borrowed.
                let list = self.constraints_of(scene, cmd)?;
                let i = Self::constraint_index(list, cmd)?;
                if remove {
                    list.remove(i);
                } else {
                    let mut c = list[i].clone();
                    constraint_opts(cmd, &mut c)?;
                    list[i] = c;
                }
                Ok(None)
            }
            "constraint_target" => {
                cmd.max_args(3)?;
                let target = cmd.arg(2, "a target object")?.text.clone();
                let sub = cmd.opt("subtarget").map(|t| t.text).unwrap_or_default();
                let weight = cmd.f("weight")?.unwrap_or(1.0);
                let list = self.constraints_of(scene, cmd)?;
                let i = Self::constraint_index(list, cmd)?;
                match &mut list[i].kind {
                    ConstraintKind::Armature { targets, .. } => targets.push((Target { object: target, subtarget: sub, head_tail: 0.0 }, weight)),
                    _ => return Err("constraint_target is for Armature constraints (others take target=)".into()),
                }
                Ok(None)
            }
            "constraint_set_inverse" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                let i = uint(&cmd.arg(1, "a constraint index")?.text)? as usize;
                let bone = cmd.opt("bone").map(|t| t.text);
                if cmd.b("clear")?.unwrap_or(false) {
                    let list = self.constraints_of(scene, cmd)?;
                    if let Some(ConstraintKind::ChildOf { inverse, .. }) = list.get_mut(i).map(|c| &mut c.kind) {
                        *inverse = Mat4::IDENTITY;
                        return Ok(None);
                    }
                    return Err(format!("constraint {i} is not a Child Of constraint"));
                }
                if !scene.child_of_set_inverse(o, bone.as_deref(), i) {
                    return Err(format!("constraint {i} is not a Child Of constraint"));
                }
                Ok(None)
            }
            "parent_with_armature" => {
                cmd.max_args(2)?;
                let child = self.object(scene, cmd.arg(0, "a mesh object")?)?;
                let arm = self.object(scene, cmd.arg(1, "an armature object")?)?;
                let mut w = ArmatureWeights::Automatic;
                pick(cmd, "weights", &WEIGHTS, &mut w)?;
                if !scene.parent_with_armature(child, arm, w) {
                    return Err("parent_with_armature needs a mesh object and a different armature object".into());
                }
                Ok(None)
            }
            other => Err(format!("unknown command '{other}'")),
        }
    }

    /// `parent CHILD PARENT bone=NAME type=object|armature|bone`.
    pub(super) fn parent_command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<()> {
        cmd.max_args(2)?;
        let child = self.object(scene, cmd.arg(0, "a child object")?)?;
        let p = cmd.arg(1, "a parent object or none")?.clone();
        let keep = cmd.b("keep_transform")?.unwrap_or(false);
        let bone = cmd.opt("bone").map(|t| t.text).filter(|b| !b.is_empty());
        let mut ty = if bone.is_some() { ParentType::Bone } else { ParentType::Object };
        pick(cmd, "type", &PARENT_TYPES, &mut ty)?;
        let world = scene.world_matrix(child);
        self.set_parent(scene, child, &p, false)?;
        let parent = scene.objects[child].parent;
        if parent.is_none() {
            ty = ParentType::Object;
        }
        if ty == ParentType::Bone {
            let b = bone.as_deref().ok_or("type=bone needs bone=")?;
            let has = parent.and_then(|pi| scene.armature_of(pi)).is_some_and(|a| a.find(b).is_some());
            if !has {
                return Err(format!("the parent is not an armature with a bone named '{b}'"));
            }
        }
        let o = &mut scene.objects[child];
        o.parent_type = ty;
        o.parent_bone = if ty == ParentType::Bone { bone.unwrap_or_default() } else { String::new() };
        if keep {
            let pm = scene.parent_matrix_depth(child, 0).unwrap_or(Mat4::IDENTITY);
            scene.objects[child].transform = Transform::from_matrix(&(pm.inverse() * world));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Writing

fn vec_list(v: &[Vec3], b64: bool) -> String {
    let flat: Vec<f32> = v.iter().flat_map(|p| p.to_array()).collect();
    if b64 {
        b64_f32(&flat)
    } else {
        fmt_list(&flat)
    }
}

/// Armature and lattice data, before the objects.
pub(super) fn write_data(o: &mut String, scene: &Scene) {
    for (arm, name) in scene.armatures.iter().zip(&scene.armature_names) {
        let an = quote(name);
        let _ = writeln!(o, "armature_new {an}");
        let def = Bone::new("", Vec3::ZERO, Vec3::ZERO);
        let mut late = Vec::new();
        for (i, bn) in arm.bones.iter().enumerate() {
            let mut l = format!("bone {an} {} head={} tail={}", quote(&bn.name), fmt_list(&bn.head.to_array()), fmt_list(&bn.tail.to_array()));
            if bn.roll != 0.0 {
                let _ = write!(l, " roll={}", f(bn.roll));
            }
            match arm.parent(i) {
                Some(p) if p < i => {
                    let _ = write!(l, " parent={}", quote(&arm.bones[p].name));
                }
                Some(p) => late.push(format!("bone {an} {} parent={}", quote(&bn.name), quote(&arm.bones[p].name))),
                None => {}
            }
            let flags = [
                ("use_connect", bn.use_connect, def.use_connect),
                ("use_deform", bn.use_deform, def.use_deform),
                ("use_inherit_rotation", bn.use_inherit_rotation, def.use_inherit_rotation),
                ("use_local_location", bn.use_local_location, def.use_local_location),
                ("use_relative_parent", bn.use_relative_parent, def.use_relative_parent),
                ("use_envelope_multiply", bn.use_envelope_multiply, def.use_envelope_multiply),
            ];
            for (k, v, d) in flags {
                if v != d {
                    let _ = write!(l, " {k}={}", b(v));
                }
            }
            if bn.inherit_scale != def.inherit_scale {
                let _ = write!(l, " inherit_scale={}", lname(&INHERIT, bn.inherit_scale));
            }
            let nums = [
                ("envelope_distance", bn.envelope_distance, def.envelope_distance),
                ("envelope_weight", bn.envelope_weight, def.envelope_weight),
                ("head_radius", bn.head_radius, def.head_radius),
                ("tail_radius", bn.tail_radius, def.tail_radius),
            ];
            for (k, v, d) in nums {
                if v != d {
                    let _ = write!(l, " {k}={}", f(v));
                }
            }
            if bn.bbone_segments != 1 {
                let _ = write!(l, " bbone_segments={}", bn.bbone_segments);
            }
            let _ = writeln!(o, "{l}");
        }
        for l in late {
            let _ = writeln!(o, "{l}");
        }
        for c in &arm.collections {
            let members: String = arm.bones.iter().filter(|bn| bn.collections.contains(c)).map(|bn| format!(" {}", quote(&bn.name))).collect();
            let _ = writeln!(o, "bone_collection {an} {}{members}", quote(c));
        }
    }
    for (l, name) in scene.lattices.iter().zip(&scene.lattice_names) {
        let interp = [l.interpolation_u, l.interpolation_v, l.interpolation_w].iter().map(|i| lname(&INTERP, *i)).collect::<Vec<_>>().join(",");
        let _ = write!(o, "lattice_new {} points={},{},{} interpolation={interp} use_outside={}", quote(name), l.points_u, l.points_v, l.points_w, b(l.use_outside));
        if l.points != l.rest_points() {
            let _ = write!(o, " verts={}", vec_list(&l.points, l.points.len() > BASE64_ABOVE));
        }
        o.push('\n');
    }
}

/// Vertex groups and shape keys of mesh `name`, after its `mesh_new`.
pub(super) fn write_mesh_extras(o: &mut String, name: &str, m: &Mesh) {
    let mn = quote(name);
    let b64 = m.positions.len() > BASE64_ABOVE;
    for (g, gname) in m.vertex_groups.iter().enumerate() {
        let mut idx = Vec::new();
        let mut w = Vec::new();
        for v in 0..m.positions.len() {
            if let Some(x) = m.weight_opt(v, g) {
                idx.push(v as u32);
                w.push(x);
            }
        }
        let _ = write!(o, "vgroup {mn} {}", quote(gname));
        if !idx.is_empty() {
            let big = idx.len() > BASE64_ABOVE;
            let it = if big { b64_u32(&idx) } else { idx.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",") };
            let wt = if big { b64_f32(&w) } else { fmt_list(&w) };
            let _ = write!(o, " indices={it} weights={wt}");
        }
        o.push('\n');
    }
    if m.shape_keys.is_empty() {
        return;
    }
    let s = &m.shape_key_settings;
    if *s != crate::mesh::ShapeKeySettings::default() {
        let _ = writeln!(o, "shape_key_settings {mn} use_relative={} eval_time={}", b(s.use_relative), f(s.eval_time));
    }
    let n = m.positions.len();
    for k in &m.shape_keys {
        let mut pos = k.positions.clone();
        pos.truncate(n);
        pos.extend_from_slice(&m.positions[pos.len()..]);
        let _ = writeln!(
            o,
            "shape_key {mn} {} verts={} value={} slider_min={} slider_max={} vertex_group={} mute={} interpolation={} frame={}",
            quote(&k.name),
            vec_list(&pos, b64),
            f(k.value),
            f(k.slider_min),
            f(k.slider_max),
            qname(&k.vertex_group),
            b(k.mute),
            lname(&INTERP, k.interpolation),
            f(k.frame)
        );
    }
    for k in &m.shape_keys {
        if k.relative_key != 0 {
            if let Some(r) = m.shape_keys.get(k.relative_key) {
                let _ = writeln!(o, "shape_key {mn} {} relative_key={}", quote(&k.name), quote(&r.name));
            }
        }
    }
}

/// The creating line of an armature or lattice object (None for others).
pub(super) fn object_line(scene: &Scene, ob: &Object, place: &str) -> Option<String> {
    let name = quote(&ob.name);
    match ob.data {
        ObjectData::Armature(a) => Some(format!("add_armature {name} data={}{place}", quote(&scene.armature_names[a]))),
        ObjectData::Lattice(l) => Some(format!("add_lattice {name} data={}{place}", quote(&scene.lattice_names[l]))),
        _ => None,
    }
}

/// The `parent` line's extra options.
pub(super) fn parent_options(ob: &Object) -> String {
    match ob.parent_type {
        ParentType::Object => String::new(),
        ParentType::Armature => " type=armature".into(),
        ParentType::Bone => format!(" bone={}", quote(&ob.parent_bone)),
    }
}

fn write_constraints(o: &mut String, obj: &str, bone: Option<&str>, list: &[Constraint]) {
    let bopt = bone.map(|b| format!(" bone={}", quote(b))).unwrap_or_default();
    for (i, c) in list.iter().enumerate() {
        let _ = writeln!(o, "constraint_add {obj} {}{bopt}{}", c.kind.type_id().to_ascii_lowercase(), constraint_to_options(c));
        if let ConstraintKind::Armature { targets, .. } = &c.kind {
            for (t, w) in targets {
                let _ = writeln!(o, "constraint_target {obj} {i} {} subtarget={} weight={}{bopt}", quote(&t.object), quote(&t.subtarget), f(*w));
            }
        }
    }
}

/// Pose and constraints of an object, after every object exists.
pub(super) fn write_object_rig(o: &mut String, ob: &Object) {
    let name = quote(&ob.name);
    let def = PoseBone::new("");
    for pb in &ob.pose {
        let mut l = format!("pose_bone {name} {}", quote(&pb.name));
        if pb.location != def.location {
            let _ = write!(l, " location={}", fmt_list(&pb.location.to_array()));
        }
        if pb.rotation_mode != def.rotation_mode {
            let _ = write!(l, " rotation_mode={}", pb.rotation_mode.name().to_ascii_lowercase());
        }
        let q = pb.rotation_quaternion;
        if q != def.rotation_quaternion {
            let _ = write!(l, " quaternion={}", fmt_list(&[q.w, q.x, q.y, q.z]));
        }
        if pb.rotation_euler != def.rotation_euler {
            let _ = write!(l, " euler={}", fmt_list(&pb.rotation_euler.to_array()));
        }
        if pb.rotation_axis_angle != def.rotation_axis_angle {
            let _ = write!(l, " axis_angle={}", fmt_list(&pb.rotation_axis_angle));
        }
        if pb.scale != def.scale {
            let _ = write!(l, " scale={}", fmt_list(&pb.scale.to_array()));
        }
        let (ik, d) = (&pb.ik, &def.ik);
        if ik.lock != d.lock {
            let _ = write!(l, " ik_lock={}", fb3(&ik.lock));
        }
        if ik.stiffness != d.stiffness {
            let _ = write!(l, " ik_stiffness={}", fmt_list(&ik.stiffness));
        }
        if ik.stretch != d.stretch {
            let _ = write!(l, " ik_stretch={}", f(ik.stretch));
        }
        if ik.use_limit != d.use_limit {
            let _ = write!(l, " ik_limit={}", fb3(&ik.use_limit));
        }
        if ik.min != d.min {
            let _ = write!(l, " ik_min={}", fmt_list(&ik.min));
        }
        if ik.max != d.max {
            let _ = write!(l, " ik_max={}", fmt_list(&ik.max));
        }
        let _ = writeln!(o, "{l}");
        write_constraints(o, &name, Some(&pb.name), &pb.constraints);
    }
    write_constraints(o, &name, None, &ob.constraints);
}
