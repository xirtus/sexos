// SPDX-License-Identifier: GPL-3.0-only
//! Object and bone constraints, and the pose solve (Blender's
//! `constraint.cc`, `armature_update.cc` and the legacy "Standard" IK
//! solver).
//!
//! A [`Constraint`] has Blender's common settings (name, influence, mute,
//! owner and target space) and a [`ConstraintKind`] with the type's options,
//! named like Blender's Python API. [`solve`] is `BKE_constraints_solve`:
//! each constraint converts the owner's world matrix into its owner space,
//! fetches its targets (converted into the target space), evaluates, converts
//! back to world space and blends with the previous result by influence
//! (`interp_m4_m4m4`). Supported spaces are World, Pose, Local With Parent,
//! Local and Custom (Owner Local is read as Local).
//!
//! Object constraints run inside [`Scene::world_matrix`] after parenting, so
//! children follow the constrained matrix. Bone constraints run inside
//! [`Scene::pose_matrices`]: bones are evaluated parents first
//! (`BKE_pose_where_is_bone`); a bone constraint targeting a later bone of
//! the same armature sees that bone's rest matrix.
//!
//! **IK** (`Inverse Kinematics`) is solved after the forward pass, owner by
//! owner, the way the legacy solver works on its trees: the chain is the
//! owner (or its parent without Use Tail) and `chain_count - 1` parents (all
//! of them for 0). Each chain bone gets an extra rotation in its own frame,
//! found by damped least squares on the positional Jacobian (axis × lever
//! for each unlocked local axis, scaled by `1 - stiffness`), clamped to the
//! bone's IK limits as swing (X, Z) and twist (Y). A pole target rotates the
//! chain about the root-to-effector line so the root's X axis (turned by
//! `pole_angle` towards Z) points at the pole, before and after solving
//! (`IK_QJacobianSolver::ConstrainPoleVector`). With Stretch, an unreachable
//! target scales chain bones along Y in proportion to their IK Stretch.
//! Influence blends the solved rotations with the forward pose. The chain
//! root's descendants are then re-evaluated. Not done: rotation targets
//! (Use Rotation), targetless (auto) IK and multiple IK trees sharing bones
//! being solved jointly.
//!
//! Not implemented: Clamp To (no curves yet), Spline IK, Action, Follow
//! Path, Shrinkwrap, Pivot, Follow Track and friends. Limit Distance and
//! Stretch To with a stored distance of 0 use the current distance (Blender
//! writes the distance back on first evaluation).
use crate::armature::{Armature, BoneParentTransform, DualQuat};
use crate::math::{EulerOrder, Mat3, Mat4, Quat, Vec3};
use crate::scene::{ObjectData, ParentType, Scene};

/// Constraint spaces (Blender's `CONSTRAINT_SPACE_*`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Space {
    #[default]
    World,
    Pose,
    LocalWithParent,
    Local,
    Custom,
}

impl Space {
    pub const NAMES: [(&'static str, Space); 5] = [("WORLD", Space::World), ("POSE", Space::Pose), ("LOCAL_WITH_PARENT", Space::LocalWithParent), ("LOCAL", Space::Local), ("CUSTOM", Space::Custom)];
    /// Blender's numbers: 0 world, 1 local, 2 pose, 3 local with parent,
    /// 4 owner local, 5 custom.
    pub fn from_blender(v: i64) -> Space {
        match v {
            1 | 4 => Space::Local,
            2 => Space::Pose,
            3 => Space::LocalWithParent,
            5 => Space::Custom,
            _ => Space::World,
        }
    }
}

/// Track axes (Blender's `TRACK_X` .. `TRACK_nZ`, 0..5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TrackAxis {
    X,
    #[default]
    Y,
    Z,
    NegX,
    NegY,
    NegZ,
}

impl TrackAxis {
    pub const NAMES: [(&'static str, TrackAxis); 6] =
        [("TRACK_X", TrackAxis::X), ("TRACK_Y", TrackAxis::Y), ("TRACK_Z", TrackAxis::Z), ("TRACK_NEGATIVE_X", TrackAxis::NegX), ("TRACK_NEGATIVE_Y", TrackAxis::NegY), ("TRACK_NEGATIVE_Z", TrackAxis::NegZ)];
    pub fn from_blender(v: i64) -> TrackAxis {
        match v {
            0 => TrackAxis::X,
            2 => TrackAxis::Z,
            3 => TrackAxis::NegX,
            4 => TrackAxis::NegY,
            5 => TrackAxis::NegZ,
            _ => TrackAxis::Y,
        }
    }
    pub fn index(self) -> usize {
        self as usize
    }
    /// Axis 0..2 and whether it is negative.
    pub fn axis(self) -> (usize, bool) {
        let i = self as usize;
        (i % 3, i >= 3)
    }
    pub fn vector(self) -> Vec3 {
        let (a, neg) = self.axis();
        let v = [Vec3::X, Vec3::Y, Vec3::Z][a];
        if neg {
            -v
        } else {
            v
        }
    }
}

/// A constraint target: an object, optionally one of its bones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    pub object: String,
    /// Bone name (empty: the object itself).
    pub subtarget: String,
    /// Position along the bone, 0 head .. 1 tail.
    pub head_tail: f32,
}

impl Target {
    pub fn object(name: &str) -> Target {
        Target { object: name.into(), ..Default::default() }
    }
    pub fn bone(object: &str, bone: &str) -> Target {
        Target { object: object.into(), subtarget: bone.into(), head_tail: 0.0 }
    }
    pub fn is_set(&self) -> bool {
        !self.object.is_empty()
    }
}

/// Copy Rotation's mix modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RotationMix {
    #[default]
    Replace,
    /// Blender's "Add" (legacy offset).
    Add,
    Before,
    After,
}

/// Copy Transforms' mix modes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformMix {
    #[default]
    Replace,
    BeforeFull,
    Before,
    BeforeSplit,
    AfterFull,
    After,
    AfterSplit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LimitDistanceMode {
    #[default]
    Inside,
    Outside,
    OnSurface,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VolumeMode {
    #[default]
    Strict,
    Uniform,
    SingleAxis,
}

/// Stretch To's volume handling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StretchVolume {
    #[default]
    XZ,
    X,
    Z,
    None,
}

/// Stretch To's kept plane.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StretchPlane {
    #[default]
    X,
    SwingY,
    Z,
}

/// Transformation constraint channels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformChannel {
    #[default]
    Location,
    Rotation,
    Scale,
}

/// How the Transformation constraint combines its output with the owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransformationMix {
    #[default]
    Replace,
    Add,
    Before,
    After,
    Multiply,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstraintKind {
    CopyLocation { target: Target, use_xyz: [bool; 3], invert: [bool; 3], use_offset: bool },
    CopyRotation { target: Target, use_xyz: [bool; 3], invert: [bool; 3], mix: RotationMix, euler_order: Option<EulerOrder> },
    CopyScale { target: Target, use_xyz: [bool; 3], power: f32, use_make_uniform: bool, use_offset: bool, use_add: bool },
    CopyTransforms { target: Target, mix: TransformMix },
    LimitDistance { target: Target, distance: f32, mode: LimitDistanceMode },
    LimitLocation { use_min: [bool; 3], min: [f32; 3], use_max: [bool; 3], max: [f32; 3] },
    /// Limits in radians.
    LimitRotation { use_limit: [bool; 3], min: [f32; 3], max: [f32; 3], euler_order: Option<EulerOrder> },
    LimitScale { use_min: [bool; 3], min: [f32; 3], use_max: [bool; 3], max: [f32; 3] },
    MaintainVolume { mode: VolumeMode, free_axis: u8, volume: f32 },
    /// Map one channel of the target to another of the owner. Ranges are for
    /// the chosen channels (rotations in radians); `map[i]` is the source axis
    /// of output axis `i`.
    Transformation {
        target: Target,
        map_from: TransformChannel,
        map_to: TransformChannel,
        from_min: [f32; 3],
        from_max: [f32; 3],
        to_min: [f32; 3],
        to_max: [f32; 3],
        map: [u8; 3],
        use_motion_extrapolate: bool,
        mix: TransformationMix,
        to_euler_order: Option<EulerOrder>,
    },
    /// `inverse` is the Set Inverse matrix (identity when not set).
    ChildOf { target: Target, use_location: [bool; 3], use_rotation: [bool; 3], use_scale: [bool; 3], inverse: Mat4 },
    TrackTo { target: Target, track_axis: TrackAxis, up_axis: u8, use_target_z: bool },
    DampedTrack { target: Target, track_axis: TrackAxis },
    LockedTrack { target: Target, track_axis: TrackAxis, lock_axis: u8 },
    StretchTo { target: Target, rest_length: f32, bulge: f32, use_bulge_min: bool, bulge_min: f32, use_bulge_max: bool, bulge_max: f32, bulge_smooth: f32, volume: StretchVolume, keep_axis: StretchPlane },
    Floor { target: Target, floor_location: TrackAxis, offset: f32, use_rotation: bool },
    /// Deform like an Armature modifier would, by the weighted bones.
    Armature { targets: Vec<(Target, f32)>, use_deform_preserve_volume: bool, use_bone_envelopes: bool },
    Ik {
        target: Target,
        pole_target: Option<Target>,
        /// Radians.
        pole_angle: f32,
        /// 0 = the whole chain to the root.
        chain_count: u32,
        iterations: u32,
        use_tail: bool,
        use_stretch: bool,
        weight: f32,
    },
}

/// One constraint on an object or a pose bone.
#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub name: String,
    pub kind: ConstraintKind,
    pub influence: f32,
    pub mute: bool,
    pub owner_space: Space,
    pub target_space: Space,
    /// Custom space object (and bone) for [`Space::Custom`].
    pub space_object: String,
    pub space_subtarget: String,
}

impl Constraint {
    pub fn new(kind: ConstraintKind) -> Constraint {
        Constraint { name: kind.name().into(), kind, influence: 1.0, mute: false, owner_space: Space::World, target_space: Space::World, space_object: String::new(), space_subtarget: String::new() }
    }

    /// A constraint of Blender type `ty` (`COPY_LOCATION`, `Copy Location`,
    /// `copylocation`, `IK`, ...) with Blender's defaults. `bone` picks the
    /// defaults that differ for bones (Track To's axes).
    pub fn with_defaults(ty: &str, bone: bool) -> Option<Constraint> {
        let key: String = ty.chars().filter(|c| !matches!(c, ' ' | '_' | '-')).collect::<String>().to_ascii_lowercase();
        let t = Target::default();
        let kind = match key.as_str() {
            "copylocation" => ConstraintKind::CopyLocation { target: t, use_xyz: [true; 3], invert: [false; 3], use_offset: false },
            "copyrotation" => ConstraintKind::CopyRotation { target: t, use_xyz: [true; 3], invert: [false; 3], mix: RotationMix::Replace, euler_order: None },
            "copyscale" => ConstraintKind::CopyScale { target: t, use_xyz: [true; 3], power: 1.0, use_make_uniform: false, use_offset: false, use_add: false },
            "copytransforms" => ConstraintKind::CopyTransforms { target: t, mix: TransformMix::Replace },
            "limitdistance" => ConstraintKind::LimitDistance { target: t, distance: 0.0, mode: LimitDistanceMode::Inside },
            "limitlocation" => ConstraintKind::LimitLocation { use_min: [false; 3], min: [0.0; 3], use_max: [false; 3], max: [0.0; 3] },
            "limitrotation" => ConstraintKind::LimitRotation { use_limit: [false; 3], min: [0.0; 3], max: [0.0; 3], euler_order: None },
            "limitscale" => ConstraintKind::LimitScale { use_min: [false; 3], min: [0.0; 3], use_max: [false; 3], max: [0.0; 3] },
            "maintainvolume" | "samevolume" => ConstraintKind::MaintainVolume { mode: VolumeMode::Strict, free_axis: 1, volume: 1.0 },
            "transformation" | "transform" => ConstraintKind::Transformation {
                target: t,
                map_from: TransformChannel::Location,
                map_to: TransformChannel::Location,
                from_min: [0.0; 3],
                from_max: [0.0; 3],
                to_min: [0.0; 3],
                to_max: [0.0; 3],
                map: [0, 1, 2],
                use_motion_extrapolate: false,
                mix: TransformationMix::Add,
                to_euler_order: None,
            },
            "childof" => ConstraintKind::ChildOf { target: t, use_location: [true; 3], use_rotation: [true; 3], use_scale: [true; 3], inverse: Mat4::IDENTITY },
            "trackto" => {
                if bone {
                    ConstraintKind::TrackTo { target: t, track_axis: TrackAxis::Y, up_axis: 2, use_target_z: false }
                } else {
                    ConstraintKind::TrackTo { target: t, track_axis: TrackAxis::NegZ, up_axis: 1, use_target_z: false }
                }
            }
            "dampedtrack" => ConstraintKind::DampedTrack { target: t, track_axis: TrackAxis::Y },
            "lockedtrack" => ConstraintKind::LockedTrack { target: t, track_axis: TrackAxis::Y, lock_axis: 2 },
            "stretchto" => ConstraintKind::StretchTo {
                target: t,
                rest_length: 0.0,
                bulge: 1.0,
                use_bulge_min: false,
                bulge_min: 0.0,
                use_bulge_max: false,
                bulge_max: 0.0,
                bulge_smooth: 0.0,
                volume: StretchVolume::XZ,
                keep_axis: StretchPlane::X,
            },
            "floor" => ConstraintKind::Floor { target: t, floor_location: TrackAxis::Z, offset: 0.0, use_rotation: false },
            "armature" => ConstraintKind::Armature { targets: Vec::new(), use_deform_preserve_volume: false, use_bone_envelopes: false },
            "ik" | "inversekinematics" => ConstraintKind::Ik { target: t, pole_target: None, pole_angle: 0.0, chain_count: 0, iterations: 500, use_tail: true, use_stretch: true, weight: 1.0 },
            _ => return None,
        };
        Some(Constraint::new(kind))
    }

    /// The main target, if the type has one.
    pub fn target(&self) -> Option<&Target> {
        use ConstraintKind::*;
        match &self.kind {
            CopyLocation { target, .. }
            | CopyRotation { target, .. }
            | CopyScale { target, .. }
            | CopyTransforms { target, .. }
            | LimitDistance { target, .. }
            | Transformation { target, .. }
            | ChildOf { target, .. }
            | TrackTo { target, .. }
            | DampedTrack { target, .. }
            | LockedTrack { target, .. }
            | StretchTo { target, .. }
            | Floor { target, .. }
            | Ik { target, .. } => Some(target),
            Armature { targets, .. } => targets.first().map(|t| &t.0),
            _ => None,
        }
    }

    pub fn target_mut(&mut self) -> Option<&mut Target> {
        use ConstraintKind::*;
        match &mut self.kind {
            CopyLocation { target, .. }
            | CopyRotation { target, .. }
            | CopyScale { target, .. }
            | CopyTransforms { target, .. }
            | LimitDistance { target, .. }
            | Transformation { target, .. }
            | ChildOf { target, .. }
            | TrackTo { target, .. }
            | DampedTrack { target, .. }
            | LockedTrack { target, .. }
            | StretchTo { target, .. }
            | Floor { target, .. }
            | Ik { target, .. } => Some(target),
            Armature { targets, .. } => targets.first_mut().map(|t| &mut t.0),
            _ => None,
        }
    }

    /// Point references to object `old` at `new` (after a rename).
    pub fn rename_object_ref(&mut self, old: &str, new: &str) {
        let fix = |t: &mut Target| {
            if t.object == old {
                t.object = new.to_string();
            }
        };
        if let Some(t) = self.target_mut() {
            fix(t);
        }
        match &mut self.kind {
            ConstraintKind::Ik { pole_target: Some(p), .. } => fix(p),
            ConstraintKind::Armature { targets, .. } => targets.iter_mut().for_each(|t| fix(&mut t.0)),
            _ => {}
        }
        if self.space_object == old {
            self.space_object = new.to_string();
        }
    }
}

impl ConstraintKind {
    /// Blender's UI name ("Copy Location").
    pub fn name(&self) -> &'static str {
        use ConstraintKind::*;
        match self {
            CopyLocation { .. } => "Copy Location",
            CopyRotation { .. } => "Copy Rotation",
            CopyScale { .. } => "Copy Scale",
            CopyTransforms { .. } => "Copy Transforms",
            LimitDistance { .. } => "Limit Distance",
            LimitLocation { .. } => "Limit Location",
            LimitRotation { .. } => "Limit Rotation",
            LimitScale { .. } => "Limit Scale",
            MaintainVolume { .. } => "Maintain Volume",
            Transformation { .. } => "Transformation",
            ChildOf { .. } => "Child Of",
            TrackTo { .. } => "Track To",
            DampedTrack { .. } => "Damped Track",
            LockedTrack { .. } => "Locked Track",
            StretchTo { .. } => "Stretch To",
            Floor { .. } => "Floor",
            Armature { .. } => "Armature",
            Ik { .. } => "IK",
        }
    }
    /// Blender's type identifier (`COPY_LOCATION`).
    pub fn type_id(&self) -> &'static str {
        use ConstraintKind::*;
        match self {
            CopyLocation { .. } => "COPY_LOCATION",
            CopyRotation { .. } => "COPY_ROTATION",
            CopyScale { .. } => "COPY_SCALE",
            CopyTransforms { .. } => "COPY_TRANSFORMS",
            LimitDistance { .. } => "LIMIT_DISTANCE",
            LimitLocation { .. } => "LIMIT_LOCATION",
            LimitRotation { .. } => "LIMIT_ROTATION",
            LimitScale { .. } => "LIMIT_SCALE",
            MaintainVolume { .. } => "MAINTAIN_VOLUME",
            Transformation { .. } => "TRANSFORM",
            ChildOf { .. } => "CHILD_OF",
            TrackTo { .. } => "TRACK_TO",
            DampedTrack { .. } => "DAMPED_TRACK",
            LockedTrack { .. } => "LOCKED_TRACK",
            StretchTo { .. } => "STRETCH_TO",
            Floor { .. } => "FLOOR",
            Armature { .. } => "ARMATURE",
            Ik { .. } => "IK",
        }
    }
}

// ---------------------------------------------------------------------------
// Spaces

/// What a space conversion needs to know about the matrix's owner.
#[derive(Clone, Copy, Debug)]
pub struct SpaceInfo {
    /// Bones: the armature object's world matrix.
    pub armature_world: Mat4,
    /// Bones: rest matrix and parent transform (from the parent's pose).
    pub bone: Option<(Mat4, BoneParentTransform)>,
    /// Objects: the parent's world matrix (with bone parenting applied).
    pub parent: Option<Mat4>,
    /// Objects: the object's own loc/rot/scale matrix.
    pub basis: Mat4,
    /// The custom space matrix (world).
    pub custom: Mat4,
}

impl SpaceInfo {
    pub const WORLD: SpaceInfo = SpaceInfo { armature_world: Mat4::IDENTITY, bone: None, parent: None, basis: Mat4::IDENTITY, custom: Mat4::IDENTITY };
}

/// `BKE_constraint_mat_convertspace`.
pub fn convert_space(m: &Mat4, from: Space, to: Space, info: &SpaceInfo, keep_scale: bool) -> Mat4 {
    if from == to {
        return *m;
    }
    match info.bone {
        Some((arm_mat, bpt)) => match from {
            Space::World => {
                if to == Space::Custom {
                    info.custom.inverse() * *m
                } else {
                    let p = info.armature_world.inverse() * *m;
                    if matches!(to, Space::Local | Space::LocalWithParent) {
                        convert_space(&p, Space::Pose, to, info, keep_scale)
                    } else {
                        p
                    }
                }
            }
            Space::Pose => match to {
                Space::Local => bpt.inverse().apply(m),
                Space::LocalWithParent => arm_mat.inverse() * *m,
                _ => {
                    let w = info.armature_world * *m;
                    convert_space(&w, Space::World, to, info, keep_scale)
                }
            },
            Space::Local => {
                let p = bpt.apply(m);
                convert_space(&p, Space::Pose, to, info, keep_scale)
            }
            Space::LocalWithParent => {
                let p = arm_mat * *m;
                convert_space(&p, Space::Pose, to, info, keep_scale)
            }
            Space::Custom => {
                let w = info.custom * *m;
                convert_space(&w, Space::World, to, info, keep_scale)
            }
        },
        None => {
            // Objects: World, Local and Custom (Pose spaces read as World).
            let norm = |s: Space| if matches!(s, Space::Pose | Space::LocalWithParent) { Space::World } else { s };
            let (from, to) = (norm(from), norm(to));
            if from == to {
                return *m;
            }
            let local_frame = || match info.parent {
                Some(p) => p,
                None => {
                    let mut d = info.basis;
                    if !keep_scale {
                        d = d.with_mat3(d.mat3().normalized());
                    }
                    d.set_col3(3, Vec3::ZERO);
                    d
                }
            };
            // To world first.
            let w = match from {
                Space::Local => local_frame() * *m,
                Space::Custom => info.custom * *m,
                _ => *m,
            };
            match to {
                Space::Local => local_frame().inverse() * w,
                Space::Custom => info.custom.inverse() * w,
                _ => w,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Solving

/// What a constraint solve is being done for.
#[derive(Clone, Copy)]
pub struct Owner<'a> {
    pub object: usize,
    /// Bone owner: the bone index and the pose matrices computed so far for
    /// this armature (targets on the same armature read them).
    pub bone: Option<(usize, &'a [Mat4])>,
    pub info: SpaceInfo,
    /// The owner's Euler order (`cob->rotOrder`).
    pub rot_order: EulerOrder,
}

/// Scene access for target lookups, with a recursion depth so objects that
/// constrain each other terminate.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub scene: &'a Scene,
    pub depth: u32,
}

/// A resolved target: its matrix in the target space (and in world space).
#[derive(Clone, Copy, Debug)]
struct Resolved {
    m: Mat4,
    world: Mat4,
    rot_order: EulerOrder,
}

impl Ctx<'_> {
    /// Pose matrices of armature object `obj`, using `own` when it is the
    /// owner's armature.
    fn poses(&self, obj: usize, owner: &Owner) -> Option<Vec<Mat4>> {
        if owner.bone.is_some() && owner.object == obj {
            return owner.bone.map(|b| b.1.to_vec());
        }
        if self.depth >= Scene::MAX_EVAL_DEPTH {
            return None;
        }
        Some(self.scene.pose_matrices_depth(obj, self.depth + Scene::REF_DEPTH))
    }

    fn world(&self, obj: usize, owner: &Owner) -> Mat4 {
        if owner.bone.is_some() && owner.object == obj {
            return owner.info.armature_world;
        }
        if owner.object == obj || self.depth >= Scene::MAX_EVAL_DEPTH {
            // Our own world matrix is in progress: use the unconstrained one.
            return self.scene.local_world_matrix(obj, self.depth + Scene::REF_DEPTH);
        }
        self.scene.world_matrix_depth(obj, self.depth + Scene::REF_DEPTH)
    }

    /// `constraint_target_to_mat4` + conversion into `space`.
    fn target(&self, t: &Target, space: Space, owner: &Owner) -> Option<Resolved> {
        if !t.is_set() {
            return None;
        }
        let s = self.scene;
        let obj = s.find_object(&t.object)?;
        let world = self.world(obj, owner);
        if t.subtarget.is_empty() {
            if owner.bone.is_none() && obj == owner.object {
                return None;
            }
            let info = self.object_space_info(obj, owner);
            let m = convert_space(&world, Space::World, space, &info, false);
            return Some(Resolved { m, world, rot_order: EulerOrder::XYZ });
        }
        let ObjectData::Armature(ai) = s.objects[obj].data else {
            let info = self.object_space_info(obj, owner);
            return Some(Resolved { m: convert_space(&world, Space::World, space, &info, false), world, rot_order: EulerOrder::XYZ });
        };
        let arm = s.armatures.get(ai)?;
        let b = arm.find(&t.subtarget)?;
        if owner.bone.is_some_and(|(ob, _)| ob == b) && obj == owner.object {
            return None;
        }
        let poses = self.poses(obj, owner)?;
        let pose = *poses.get(b)?;
        let mut pm = pose;
        if t.head_tail > 1e-6 {
            let head = pose.translation_part();
            let tail = head + pose.col3(1).normalize() * arm.bones[b].length() * pose.col3(1).length();
            pm.set_col3(3, head.lerp(tail, t.head_tail));
        }
        let w = world * pm;
        let info = bone_space_info(arm, b, &poses, world);
        let rot_order = crate::armature::pose_bone(&s.objects[obj].pose, &t.subtarget).map_or(EulerOrder::XYZ, |p| p.rotation_mode.euler_order());
        Some(Resolved { m: convert_space(&w, Space::World, space, &info, false), world: w, rot_order })
    }

    fn object_space_info(&self, obj: usize, owner: &Owner) -> SpaceInfo {
        let s = self.scene;
        let parent = if self.depth >= Scene::MAX_EVAL_DEPTH || s.objects[obj].parent == Some(owner.object) { None } else { s.parent_matrix_depth(obj, self.depth + Scene::REF_DEPTH) };
        SpaceInfo { parent, basis: s.objects[obj].transform.matrix(), ..SpaceInfo::WORLD }
    }

    fn custom_space(&self, c: &Constraint, owner: &Owner) -> Mat4 {
        if c.owner_space != Space::Custom && c.target_space != Space::Custom {
            return Mat4::IDENTITY;
        }
        let t = Target { object: c.space_object.clone(), subtarget: c.space_subtarget.clone(), head_tail: 0.0 };
        self.target(&t, Space::World, owner).map_or(Mat4::IDENTITY, |r| r.world)
    }
}

/// Space info of bone `b` of `arm` with pose `poses`.
pub fn bone_space_info(arm: &Armature, b: usize, poses: &[Mat4], armature_world: Mat4) -> SpaceInfo {
    let pp = arm.parent(b).and_then(|p| poses.get(p)).copied();
    let bpt = arm.parent_transform(b, pp.as_ref());
    SpaceInfo { armature_world, bone: Some((arm.bones[b].rest_matrix(), bpt)), ..SpaceInfo::WORLD }
}

/// `BKE_constraints_solve`: apply `list` to the world matrix `m`. IK
/// constraints are skipped (the pose solve handles them).
pub fn solve(list: &[Constraint], m: &mut Mat4, owner: &Owner, ctx: &Ctx) {
    for c in list {
        if c.mute || c.influence <= 0.0 || matches!(c.kind, ConstraintKind::Ik { .. }) {
            continue;
        }
        let mut info = owner.info;
        info.custom = ctx.custom_space(c, owner);
        let owner_here = Owner { info, ..*owner };
        let old = *m;
        let mut local = convert_space(m, Space::World, c.owner_space, &info, false);
        if evaluate(c, &mut local, &owner_here, ctx) {
            *m = convert_space(&local, c.owner_space, Space::World, &info, false);
            if c.influence < 1.0 {
                *m = old.interpolate(m, c.influence);
            }
        }
    }
}

fn axis_index(v: u8) -> usize {
    (v as usize).min(2)
}

/// Blender's `compatible_eul`: bring each angle within π of `old`.
fn compatible_euler(e: &mut [f32; 3], old: &[f32; 3]) {
    let tau = std::f32::consts::TAU;
    for i in 0..3 {
        let d = e[i] - old[i];
        if d.abs() > std::f32::consts::PI {
            e[i] -= (d / tau).round() * tau;
        }
    }
}

/// Blender's `vectomat` (Track To).
fn vectomat(vec: Vec3, target_up: Vec3, axis: usize, upflag: usize, use_target_z: bool) -> Mat3 {
    let mut n = vec.normalize();
    if n == Vec3::ZERO {
        n = Vec3::Z;
    }
    let axis = if axis > 2 {
        axis - 3
    } else {
        n = -n;
        axis
    };
    let u = if use_target_z { target_up } else { Vec3::Z };
    let proj_on_n = if n.length_squared() > 0.0 { n * (u.dot(n) / n.length_squared()) } else { Vec3::ZERO };
    let mut proj = (u - proj_on_n).normalize();
    if proj == Vec3::ZERO {
        proj = Vec3::Y;
    }
    let right = proj.cross(n).normalize();
    if axis == upflag {
        return Mat3::IDENTITY;
    }
    let right_index = 3 - axis - upflag;
    let neg = match axis as i32 - upflag as i32 {
        1 | -2 => 1.0,
        -1 | 2 => -1.0,
        _ => 0.0,
    };
    let mut m = Mat3::IDENTITY;
    m.cols[right_index] = right * neg;
    m.cols[upflag] = proj;
    m.cols[axis] = n;
    m
}

/// `mul_m4_m3m4`: replace the 3x3 block of `m` by `r * m3`.
fn premul3(r: &Mat3, m: &Mat4) -> Mat4 {
    m.with_mat3(*r * m.mat3())
}

/// Evaluate one constraint on the owner matrix in owner space. Returns false
/// when it had nothing to do (no valid target).
fn evaluate(c: &Constraint, m: &mut Mat4, owner: &Owner, ctx: &Ctx) -> bool {
    use ConstraintKind::*;
    let tgt = |t: &Target| ctx.target(t, c.target_space, owner);
    match &c.kind {
        CopyLocation { target, use_xyz, invert, use_offset } => {
            let Some(t) = tgt(target) else { return false };
            let offset = if *use_offset { m.translation_part() } else { Vec3::ZERO };
            let tl = t.m.translation_part();
            let mut l = m.translation_part();
            for a in 0..3 {
                if use_xyz[a] {
                    let mut v = tl[a];
                    if invert[a] {
                        v = -v;
                    }
                    set(&mut l, a, v + offset[a]);
                }
            }
            m.set_col3(3, l);
        }
        CopyRotation { target, use_xyz, invert, mix, euler_order } => {
            let Some(t) = tgt(target) else { return false };
            let (loc, oldrot, size) = m.to_loc_mat3_scale();
            let order = euler_order.unwrap_or(owner.rot_order);
            let obeul = m.mat3().to_euler(order);
            let tm = orthogonalize_stable_norm(&t.m.mat3());
            let mut eul = tm.to_euler(order);
            compatible_euler(&mut eul, &obeul);
            let legacy = *mix == RotationMix::Add;
            let defeul = if matches!(mix, RotationMix::Replace | RotationMix::Add) { obeul } else { [0.0; 3] };
            for a in 0..3 {
                if !use_xyz[a] {
                    eul[a] = defeul[a];
                } else {
                    if legacy {
                        eul[a] += obeul[a];
                    }
                    if invert[a] {
                        eul[a] = -eul[a];
                    }
                }
            }
            compatible_euler(&mut eul, &obeul);
            let mut newrot = Mat3::from_euler(eul, order);
            match mix {
                RotationMix::Before => newrot = newrot * oldrot,
                RotationMix::After => newrot = oldrot * newrot,
                _ => {}
            }
            *m = Mat4::from_loc_mat3_scale(loc, newrot, size);
        }
        CopyScale { target, use_xyz, power, use_make_uniform, use_offset, use_add } => {
            let Some(t) = tgt(target) else { return false };
            let mut size = t.m.scale_part();
            let obsize = m.scale_part();
            if *use_make_uniform {
                let total = if use_xyz.iter().all(|&b| b) {
                    t.m.mat3().determinant().abs()
                } else {
                    (0..3).filter(|&a| use_xyz[a]).map(|a| size[a]).product()
                };
                size = Vec3::splat(total.cbrt());
            }
            if *power != 1.0 {
                size = Vec3::new(size.x.powf(*power), size.y.powf(*power), size.z.powf(*power));
            }
            if *use_offset {
                if *use_add {
                    size = size + obsize - Vec3::ONE;
                } else {
                    size = size.mul_elem(obsize);
                }
            }
            for a in 0..3 {
                if (use_xyz[a] || *use_make_uniform) && obsize[a] != 0.0 {
                    m.set_col3(a, m.col3(a) * (size[a] / obsize[a]));
                }
            }
        }
        CopyTransforms { target, mix } => {
            let Some(t) = tgt(target) else { return false };
            *m = match mix {
                TransformMix::Replace => t.m,
                TransformMix::BeforeFull => t.m * *m,
                TransformMix::AfterFull => *m * t.m,
                TransformMix::Before => mul_aligned_scale(&t.m, m),
                TransformMix::After => mul_aligned_scale(m, &t.m),
                TransformMix::BeforeSplit => mul_split_channels(&t.m, m),
                TransformMix::AfterSplit => mul_split_channels(m, &t.m),
            };
        }
        LimitDistance { target, distance, mode } => {
            let Some(t) = tgt(target) else { return false };
            let (op, tp) = (m.translation_part(), t.m.translation_part());
            let dist = op.distance(tp);
            let limit = if *distance == 0.0 { dist } else { *distance };
            let mut sfac = 1.0;
            let clamp = match mode {
                LimitDistanceMode::Outside => {
                    if dist <= limit && dist != 0.0 {
                        sfac = limit / dist;
                    }
                    dist <= limit
                }
                LimitDistanceMode::Inside => {
                    if dist >= limit && dist != 0.0 {
                        sfac = limit / dist;
                    }
                    dist >= limit
                }
                LimitDistanceMode::OnSurface => {
                    if dist != 0.0 {
                        sfac = limit / dist;
                    }
                    (dist - limit).abs() > 1e-6
                }
            };
            if clamp {
                m.set_col3(3, tp.lerp(op, sfac));
            }
        }
        LimitLocation { use_min, min, use_max, max } => {
            let mut l = m.translation_part();
            for a in 0..3 {
                if use_min[a] && l[a] < min[a] {
                    set(&mut l, a, min[a]);
                }
                if use_max[a] && l[a] > max[a] {
                    set(&mut l, a, max[a]);
                }
            }
            m.set_col3(3, l);
        }
        LimitRotation { use_limit, min, max, euler_order } => {
            if !use_limit.iter().any(|&b| b) {
                return true;
            }
            let o = orthogonalize_stable_keep(&m.mat3());
            let loc = m.translation_part();
            let size = o.scale();
            let order = euler_order.unwrap_or(owner.rot_order);
            let mut e = o.to_euler(order);
            for a in 0..3 {
                if use_limit[a] {
                    e[a] = e[a].clamp(min[a].min(max[a]), max[a].max(min[a]));
                }
            }
            *m = Mat4::from_loc_mat3_scale(loc, Mat3::from_euler(e, order), size);
        }
        LimitScale { use_min, min, use_max, max } => {
            let obsize = m.scale_part();
            let mut size = obsize;
            for a in 0..3 {
                if use_min[a] && size[a] < min[a] {
                    set(&mut size, a, min[a]);
                }
                if use_max[a] && size[a] > max[a] {
                    set(&mut size, a, max[a]);
                }
                if obsize[a] != 0.0 {
                    m.set_col3(a, m.col3(a) * (size[a] / obsize[a]));
                }
            }
        }
        MaintainVolume { mode, free_axis, volume } => {
            let s = m.scale_part();
            let fa = axis_index(*free_axis);
            let total = match mode {
                VolumeMode::Strict => s.x * s.y * s.z,
                VolumeMode::Uniform => s[fa].powi(3),
                VolumeMode::SingleAxis => s[fa],
            };
            let fac = if total != 0.0 { (volume / total).abs().sqrt() } else { 1.0 };
            for a in 0..3 {
                if a != fa {
                    m.set_col3(a, m.col3(a) * fac);
                }
            }
        }
        Transformation { target, map_from, map_to, from_min, from_max, to_min, to_max, map, use_motion_extrapolate, mix, to_euler_order } => {
            let Some(t) = tgt(target) else { return false };
            let mut dvec = match map_from {
                TransformChannel::Scale => {
                    let mut s = t.m.scale_part();
                    if t.m.determinant() < 0.0 {
                        s = -s;
                    }
                    s
                }
                TransformChannel::Rotation => Vec3::from_array(t.m.mat3().to_euler(owner.rot_order)),
                TransformChannel::Location => t.m.translation_part(),
            };
            let mut sval = [0.0f32; 3];
            for i in 0..3 {
                if !use_motion_extrapolate {
                    let c = dvec[i].clamp(from_min[i].min(from_max[i]), from_max[i].max(from_min[i]));
                    set(&mut dvec, i, c);
                }
                let range = from_max[i] - from_min[i];
                sval[i] = if range != 0.0 { (dvec[i] - from_min[i]) / range } else { 0.0 };
            }
            let order = to_euler_order.unwrap_or(owner.rot_order);
            let (mut loc, mut rot, mut size) = m.to_loc_mat3_scale();
            let out: [f32; 3] = std::array::from_fn(|i| to_min[i] + sval[axis_index(map[i])] * (to_max[i] - to_min[i]));
            match map_to {
                TransformChannel::Scale => {
                    let ns = Vec3::from_array(out);
                    size = if *mix == TransformationMix::Multiply { size.mul_elem(ns) } else { ns };
                }
                TransformChannel::Rotation => {
                    let nr = Mat3::from_euler(out, order);
                    rot = match mix {
                        TransformationMix::Replace => nr,
                        TransformationMix::Before => nr * rot,
                        TransformationMix::After => rot * nr,
                        _ => {
                            let old = rot.to_euler(order);
                            Mat3::from_euler([out[0] + old[0], out[1] + old[1], out[2] + old[2]], order)
                        }
                    };
                }
                TransformChannel::Location => {
                    let nl = Vec3::from_array(out);
                    loc = if *mix == TransformationMix::Replace { nl } else { loc + nl };
                }
            }
            *m = Mat4::from_loc_mat3_scale(loc, rot, size);
        }
        ChildOf { target, use_location, use_rotation, use_scale, inverse } => {
            let Some(t) = tgt(target) else { return false };
            let (parmat, inv) = if use_location.iter().chain(use_rotation).chain(use_scale).all(|&b| b) {
                (t.m, *inverse)
            } else {
                let filter = |mm: &Mat4, order: EulerOrder| {
                    let mut l = mm.translation_part();
                    let mut e = mm.mat3().to_euler(order);
                    let mut s = mm.scale_part();
                    for a in 0..3 {
                        if !use_location[a] {
                            set(&mut l, a, 0.0);
                        }
                        if !use_rotation[a] {
                            e[a] = 0.0;
                        }
                        if !use_scale[a] {
                            set(&mut s, a, 1.0);
                        }
                    }
                    Mat4::from_loc_mat3_scale(l, Mat3::from_euler(e, order), s)
                };
                (filter(&t.m, t.rot_order), filter(inverse, owner.rot_order))
            };
            *m = parmat * inv * *m;
        }
        TrackTo { target, track_axis, up_axis, use_target_z } => {
            let Some(t) = tgt(target) else { return false };
            let size = m.scale_part();
            let mut base = *m;
            base = base.with_mat3(Mat3::diagonal(size));
            let vec = m.translation_part() - t.m.translation_part();
            let tot = vectomat(vec, t.m.col3(2), track_axis.index(), axis_index(*up_axis), *use_target_z);
            *m = premul3(&tot, &base);
        }
        DampedTrack { target, track_axis } => {
            let Some(t) = tgt(target) else { return false };
            damped_track(m, t.m.translation_part() - m.translation_part(), *track_axis);
        }
        LockedTrack { target, track_axis, lock_axis } => {
            let Some(t) = tgt(target) else { return false };
            let vec = t.m.translation_part() - m.translation_part();
            let (ta, neg) = track_axis.axis();
            let la = axis_index(*lock_axis);
            if ta == la {
                return true;
            }
            let lock = m.col3(la).normalize();
            let mut track = (vec - lock * vec.dot(lock)).normalize();
            if neg {
                track = -track;
            }
            let k = 3 - ta - la;
            let cyclic = (la + 1) % 3 == ta;
            let third = if cyclic { lock.cross(track) } else { track.cross(lock) };
            let mut tot = Mat3::IDENTITY;
            tot.cols[la] = lock;
            tot.cols[ta] = track;
            tot.cols[k] = third;
            let n = m.mat3().normalized();
            let mut rel = tot * n.inverse();
            if rel.determinant() == 0.0 || !rel.determinant().is_finite() {
                rel = Mat3::IDENTITY;
            }
            *m = premul3(&rel, m);
        }
        StretchTo { target, rest_length, bulge, use_bulge_min, bulge_min, use_bulge_max, bulge_max, bulge_smooth, volume, keep_axis } => {
            let Some(t) = tgt(target) else { return false };
            let size = m.scale_part();
            let n = m.mat3().normalized();
            let (xx, zz, yy) = (n.cols[0], n.cols[2], n.cols[1]);
            let d = t.m.translation_part() - m.translation_part();
            let mut dist = d.length();
            let vec = if dist > 0.0 { d / dist } else { yy };
            if size.y != 0.0 {
                dist /= size.y;
            }
            let orglength = if *rest_length == 0.0 { dist } else { *rest_length };
            if dist <= 0.0 || orglength <= 0.0 {
                return true;
            }
            let sy = dist / orglength;
            let mut b = (orglength / dist).powf(*bulge);
            if b > 1.0 && *use_bulge_max {
                let bmax = bulge_max.max(1.0);
                let hard = b.min(bmax);
                let range = bmax - 1.0;
                let sf = if range > 0.0 { 1.0 / range } else { 0.0 };
                let soft = 1.0 + range * ((b - 1.0) * sf).atan() / std::f32::consts::FRAC_PI_2;
                b = soft + (hard - soft) * bulge_smooth;
            }
            if b < 1.0 && *use_bulge_min {
                let bmin = bulge_min.clamp(0.0, 1.0);
                let hard = b.max(bmin);
                let range = 1.0 - bmin;
                let sf = if range > 0.0 { 1.0 / range } else { 0.0 };
                let soft = 1.0 - range * ((1.0 - b) * sf).atan() / std::f32::consts::FRAC_PI_2;
                b = soft + (hard - soft) * bulge_smooth;
            }
            let (sx, sz) = match volume {
                StretchVolume::XZ => (b.sqrt(), b.sqrt()),
                StretchVolume::X => (b, 1.0),
                StretchVolume::Z => (1.0, b),
                StretchVolume::None => (1.0, 1.0),
            };
            let rot = match keep_axis {
                StretchPlane::X => {
                    let orth = xx.cross(vec).normalize();
                    Mat3::from_cols(vec.cross(orth).normalize(), vec, orth)
                }
                StretchPlane::Z => {
                    let orth = zz.cross(vec).normalize();
                    Mat3::from_cols(-orth, vec, orth.cross(vec).normalize())
                }
                StretchPlane::SwingY => Mat3::from_quat(Quat::from_rotation_arc(yy, vec)) * n,
            };
            let base = m.with_mat3(Mat3::diagonal(Vec3::new(size.x * sx, size.y * sy, size.z * sz)));
            *m = premul3(&rot, &base);
        }
        Floor { target, floor_location, offset, use_rotation } => {
            let Some(t) = tgt(target) else { return false };
            let mut obmat = *m;
            let mut tarmat = t.m;
            if *use_rotation {
                obmat = t.m.inverse() * obmat;
                tarmat = Mat4::IDENTITY;
            }
            let (idx, neg) = floor_location.axis();
            let tv = tarmat.translation_part()[idx];
            let ov = obmat.translation_part()[idx];
            let (v1, v2) = if neg { (ov - offset, tv) } else { (tv, ov - offset) };
            if v1 > v2 {
                let mut l = obmat.translation_part();
                set(&mut l, idx, tv + offset);
                obmat.set_col3(3, l);
                if *use_rotation {
                    *m = t.m * obmat;
                } else {
                    m.set_col3(3, l);
                }
            }
        }
        Armature { targets, use_deform_preserve_volume, use_bone_envelopes } => {
            let input = m.translation_part();
            let mut sum = Mat4::ZERO;
            let mut dq = DualQuat::ZERO;
            let mut weight = 0.0f32;
            for (t, w) in targets {
                let Some(obj) = ctx.scene.find_object(&t.object) else { continue };
                let ObjectData::Armature(ai) = ctx.scene.objects[obj].data else { continue };
                let Some(arm) = ctx.scene.armatures.get(ai) else { continue };
                let Some(b) = arm.find(&t.subtarget) else { continue };
                let Some(poses) = ctx.poses(obj, owner) else { continue };
                let aw = ctx.world(obj, owner);
                let rest = arm.bones[b].rest_matrix();
                let chan = poses[b] * rest.inverse();
                let mut wt = *w;
                if *use_bone_envelopes {
                    let bone = &arm.bones[b];
                    let p = aw.inverse().transform_point(input);
                    wt *= crate::armature::distfactor_to_bone(p, bone.head, bone.tail, bone.head_radius, bone.tail_radius, bone.envelope_distance);
                }
                if wt <= 0.0 {
                    continue;
                }
                let basemat = aw * chan * aw.inverse();
                if *use_deform_preserve_volume {
                    dq.add_weighted(&DualQuat::from_matrices(&(aw * rest), &basemat), wt);
                } else {
                    sum = sum.add_mat(&basemat.scaled(wt));
                }
                weight += wt;
            }
            if weight <= 0.0 {
                return true;
            }
            if *use_deform_preserve_volume {
                dq.normalize(weight);
                let l = dq.transform(input);
                // Rotate by the blended quaternion as well.
                let q = Quat::new(dq.quat[0], dq.quat[1], dq.quat[2], dq.quat[3]).normalize();
                *m = premul3(&Mat3::from_quat(q), m);
                m.set_col3(3, l);
            } else {
                *m = sum.scaled(1.0 / weight) * *m;
            }
        }
        Ik { .. } => return false,
    }
    true
}

fn set(v: &mut Vec3, a: usize, x: f32) {
    match a {
        0 => v.x = x,
        1 => v.y = x,
        _ => v.z = x,
    }
}

fn orthogonalize_stable_norm(m: &Mat3) -> Mat3 {
    crate::armature::orthogonalize_stable(m, 1, true)
}

fn orthogonalize_stable_keep(m: &Mat3) -> Mat3 {
    crate::armature::orthogonalize_stable(m, 1, false)
}

/// `mul_m4_m4m4_aligned_scale`.
fn mul_aligned_scale(a: &Mat4, b: &Mat4) -> Mat4 {
    let (_, ra, sa) = a.to_loc_mat3_scale();
    let (lb, rb, sb) = b.to_loc_mat3_scale();
    Mat4::from_loc_mat3_scale(a.transform_point(lb), ra * rb, sa.mul_elem(sb))
}

/// `mul_m4_m4m4_split_channels`.
fn mul_split_channels(a: &Mat4, b: &Mat4) -> Mat4 {
    let (la, ra, sa) = a.to_loc_mat3_scale();
    let (lb, rb, sb) = b.to_loc_mat3_scale();
    Mat4::from_loc_mat3_scale(la + lb, ra * rb, sa.mul_elem(sb))
}

/// `damptrack_do_transform`.
fn damped_track(m: &mut Mat4, tarvec: Vec3, axis: TrackAxis) {
    let tv = tarvec.normalize();
    if tv == Vec3::ZERO {
        return;
    }
    let mut obvec = m.transform_vector(axis.vector()).normalize();
    if obvec == Vec3::ZERO {
        obvec = axis.vector();
    }
    let loc = m.translation_part();
    let mut raxis = obvec.cross(tv);
    let mut rangle = obvec.dot(tv).clamp(-1.0, 1.0).acos();
    let norm = raxis.length();
    if norm < f32::EPSILON {
        if rangle.abs() < std::f32::consts::PI - 0.01 {
            return;
        }
        rangle = std::f32::consts::PI;
        let other = [TrackAxis::X, TrackAxis::Y, TrackAxis::Z, TrackAxis::NegX, TrackAxis::NegY, TrackAxis::NegZ][(axis.index() + 1) % 6];
        let tmp = m.transform_vector(other.vector());
        raxis = obvec.cross(tmp);
        if raxis.normalize() == Vec3::ZERO {
            return;
        }
    } else if norm < 0.1 {
        rangle = if rangle > std::f32::consts::FRAC_PI_2 { std::f32::consts::PI - norm.asin() } else { norm.asin() };
    }
    let r = Mat3::from_axis_angle(raxis.normalize(), rangle);
    *m = premul3(&r, m);
    m.set_col3(3, loc);
}

impl Mat4 {
    pub const ZERO: Mat4 = Mat4 { cols: [[0.0; 4]; 4] };
}

// ---------------------------------------------------------------------------
// Pose solve

/// Per-bone IK results applied after the forward pose.
#[derive(Clone, Copy, Debug)]
struct IkDelta {
    rot: Quat,
    stretch: f32,
}

impl Default for IkDelta {
    fn default() -> Self {
        IkDelta { rot: Quat::IDENTITY, stretch: 1.0 }
    }
}

struct PoseSolver<'a> {
    scene: &'a Scene,
    obj: usize,
    arm: &'a Armature,
    world: Mat4,
    depth: u32,
    poses: Vec<Mat4>,
    ik: Vec<Option<IkDelta>>,
}

impl PoseSolver<'_> {
    /// `BKE_pose_where_is_bone` for bone `i` (with its constraints and any
    /// IK delta).
    fn eval_bone(&mut self, i: usize) {
        let s = self.scene;
        let b = &self.arm.bones[i];
        let pb = crate::armature::pose_bone(&s.objects[self.obj].pose, &b.name);
        let connected = b.use_connect && self.arm.parent(i).is_some();
        let chan = pb.map_or(Mat4::IDENTITY, |p| p.channel_matrix(connected));
        let pp = self.arm.parent(i).map(|p| self.poses[p]);
        let bpt = self.arm.parent_transform(i, pp.as_ref());
        let mut pose = bpt.apply(&chan);
        if let Some(pb) = pb.filter(|p| p.constraints.iter().any(|c| !c.mute && !matches!(c.kind, ConstraintKind::Ik { .. }))) {
            let head = pose.translation_part();
            let mut w = self.world * pose;
            let info = SpaceInfo { armature_world: self.world, bone: Some((b.rest_matrix(), bpt)), ..SpaceInfo::WORLD };
            let owner = Owner { object: self.obj, bone: Some((i, &self.poses)), info, rot_order: pb.rotation_mode.euler_order() };
            solve(&pb.constraints, &mut w, &owner, &Ctx { scene: s, depth: self.depth });
            pose = self.world.inverse() * w;
            if connected {
                pose.set_col3(3, head);
            }
        }
        if let Some(d) = self.ik[i] {
            pose = pose * Mat3::from_quat(d.rot).to_mat4();
            if d.stretch != 1.0 {
                pose = pose.rescale(Vec3::new(1.0, d.stretch, 1.0));
            }
        }
        self.poses[i] = pose;
    }

    fn tail(&self, i: usize) -> Vec3 {
        let p = &self.poses[i];
        p.translation_part() + p.col3(1) * self.arm.bones[i].length()
    }

    fn run(&mut self) {
        let order = self.arm.order();
        for &i in &order {
            self.eval_bone(i);
        }
        // IK constraints, owner by owner.
        let scene = self.scene;
        let pose = &scene.objects[self.obj].pose;
        for &owner in &order {
            let name = &self.arm.bones[owner].name;
            let Some(pb) = crate::armature::pose_bone(pose, name) else { continue };
            for c in &pb.constraints {
                if let ConstraintKind::Ik { target, pole_target, pole_angle, chain_count, iterations, use_tail, use_stretch, .. } = &c.kind {
                    if c.mute || c.influence <= 0.0 {
                        continue;
                    }
                    self.solve_ik(owner, c, target, pole_target.as_ref(), *pole_angle, *chain_count, *iterations, *use_tail, *use_stretch, &order);
                }
            }
        }
    }

    /// World-space target position through the generic target lookup.
    fn target_pos(&self, t: &Target, owner_bone: usize) -> Option<Vec3> {
        let info = SpaceInfo { armature_world: self.world, ..SpaceInfo::WORLD };
        let owner = Owner { object: self.obj, bone: Some((owner_bone, &self.poses)), info, rot_order: EulerOrder::XYZ };
        let ctx = Ctx { scene: self.scene, depth: self.depth };
        let r = ctx.target(t, Space::World, &owner)?;
        Some(self.world.inverse().transform_point(r.world.translation_part()))
    }

    #[allow(clippy::too_many_arguments)]
    fn solve_ik(&mut self, owner: usize, c: &Constraint, target: &Target, pole: Option<&Target>, pole_angle: f32, chain_count: u32, iterations: u32, use_tail: bool, use_stretch: bool, order: &[usize]) {
        let tip = if use_tail { Some(owner) } else { self.arm.parent(owner) };
        let Some(tip) = tip else { return };
        let Some(goal) = self.target_pos(target, owner) else { return };
        // Chain root .. tip.
        let mut chain = vec![tip];
        while chain_count == 0 || chain.len() < chain_count as usize {
            match self.arm.parent(*chain.last().unwrap()) {
                Some(p) => chain.push(p),
                None => break,
            }
        }
        chain.reverse();
        let root = chain[0];
        let scene = self.scene;
        let pose = &scene.objects[self.obj].pose;
        let settings: Vec<crate::armature::IkSettings> = chain.iter().map(|&b| crate::armature::pose_bone(pose, &self.arm.bones[b].name).map(|p| p.ik).unwrap_or_default()).collect();
        let fk: Vec<Option<IkDelta>> = chain.iter().map(|&b| self.ik[b]).collect();
        for &b in &chain {
            self.ik[b].get_or_insert_with(IkDelta::default);
        }
        let reach: f32 = chain.iter().map(|&b| self.arm.bones[b].length()).sum::<f32>().max(1e-6);
        let eval_chain = |s: &mut Self| {
            for &b in &chain {
                s.eval_bone(b);
            }
        };
        let pole_pos = pole.and_then(|p| self.target_pos(p, owner));
        // Pole: rotate the chain about the root so the root's X (turned by
        // the pole angle) faces the pole, towards `dir_goal`.
        let constrain_pole = |s: &mut Self, towards_goal: bool| {
            let Some(pp) = pole_pos else { return };
            eval_chain(s);
            let root_pos = s.poses[root].translation_part();
            let end = s.tail(tip);
            let rb = s.poses[root].mat3().normalized();
            let dir = (end - root_pos).normalize();
            let up = rb.cols[0] * pole_angle.cos() + rb.cols[2] * pole_angle.sin();
            let poledir = if towards_goal { (goal - root_pos).normalize() } else { dir };
            let poleup = (pp - root_pos).normalize();
            let rows = |d: Vec3, u: Vec3| {
                let r0 = d.cross(u).normalize();
                let r1 = r0.cross(d);
                // Rows r0, r1, -d as a matrix: transpose of the column form.
                Mat3::from_cols(r0, r1, -d).transpose()
            };
            let (mat, polemat) = (rows(dir, up), rows(poledir, poleup));
            let rw = polemat.transpose() * mat;
            if !rw.cols.iter().all(|c| c.x.is_finite() && c.y.is_finite() && c.z.is_finite()) {
                return;
            }
            let mr = s.poses[root].mat3().normalized();
            let dq = (mr.inverse() * rw * mr).to_quat();
            if let Some(d) = s.ik[root].as_mut() {
                d.rot = (d.rot * dq).normalize();
            }
        };
        constrain_pole(self, true);
        // Stretch: lengthen bones with IK Stretch when the goal is out of reach.
        if use_stretch {
            eval_chain(self);
            let root_pos = self.poses[root].translation_part();
            let dist = (goal - root_pos).length();
            let ks: f32 = chain.iter().zip(&settings).map(|(&b, s)| self.arm.bones[b].length() * s.stretch).sum();
            if dist > reach && ks > 1e-6 {
                let t = (dist - reach) / ks;
                for (&b, s) in chain.iter().zip(&settings) {
                    if let Some(d) = self.ik[b].as_mut() {
                        d.stretch = 1.0 + t * s.stretch;
                    }
                }
            }
        }
        let iters = iterations.clamp(1, 10_000);
        let lambda = 0.05 * reach;
        for _ in 0..iters {
            eval_chain(self);
            let end = self.tail(tip);
            let err = goal - end;
            if err.length() < 1e-5 * reach {
                break;
            }
            // Jacobian columns.
            let mut cols: Vec<(usize, usize, Vec3, f32)> = Vec::new();
            for (ci, &b) in chain.iter().enumerate() {
                let s = &settings[ci];
                let head = self.poses[b].translation_part();
                for a in 0..3 {
                    if s.lock[a] {
                        continue;
                    }
                    let wgt = (1.0 - s.stiffness[a]).clamp(0.0, 1.0);
                    if wgt <= 0.0 {
                        continue;
                    }
                    let axis = self.poses[b].col3(a).normalize();
                    cols.push((ci, a, axis.cross(end - head) * wgt, wgt));
                }
            }
            if cols.is_empty() {
                break;
            }
            // J Jᵀ + λ² I (3x3).
            let mut jjt = Mat3::diagonal(Vec3::splat(lambda * lambda));
            for c in &cols {
                let j = c.2;
                jjt = jjt + Mat3::from_cols(j * j.x, j * j.y, j * j.z);
            }
            let y = jjt.inverse().mul_vec(err);
            let mut max_step = 0.0f32;
            let steps: Vec<f32> = cols.iter().map(|c| c.2.dot(y) * c.3).collect();
            for s in &steps {
                max_step = max_step.max(s.abs());
            }
            let scale = if max_step > 0.5 { 0.5 / max_step } else { 1.0 };
            for (c, step) in cols.iter().zip(&steps) {
                let b = chain[c.0];
                if let Some(d) = self.ik[b].as_mut() {
                    let ax = [Vec3::X, Vec3::Y, Vec3::Z][c.1];
                    d.rot = (d.rot * Quat::from_axis_angle(ax, step * scale)).normalize();
                }
            }
            for (ci, &b) in chain.iter().enumerate() {
                if let Some(d) = self.ik[b].as_mut() {
                    d.rot = clamp_ik(d.rot, &settings[ci]);
                }
            }
        }
        constrain_pole(self, false);
        // Influence: blend with the forward pose.
        if c.influence < 1.0 {
            for (ci, &b) in chain.iter().enumerate() {
                let base = fk[ci].unwrap_or_default();
                if let Some(d) = self.ik[b].as_mut() {
                    d.rot = base.rot.slerp(d.rot, c.influence);
                    d.stretch = base.stretch + (d.stretch - base.stretch) * c.influence;
                }
            }
        }
        // Re-evaluate the chain and everything below its root.
        for &b in order {
            if self.arm.is_ancestor(root, b) {
                self.eval_bone(b);
            }
        }
    }
}

/// Clamp an IK rotation to the bone's locks and limits: swing about X/Z
/// and twist about Y, each axis in its own range.
fn clamp_ik(q: Quat, s: &crate::armature::IkSettings) -> Quat {
    if !s.lock.iter().any(|&b| b) && !s.use_limit.iter().any(|&b| b) {
        return q;
    }
    // q = swing * twist, twist about Y.
    let q = q.normalize();
    let tw = Quat { x: 0.0, y: q.y, z: 0.0, w: q.w }.normalize();
    let swing = q * tw.conjugate();
    let mut twist = 2.0 * tw.y.atan2(tw.w);
    if twist > std::f32::consts::PI {
        twist -= std::f32::consts::TAU;
    }
    let (axis, ang) = swing.to_axis_angle();
    let mut sx = axis.x * ang;
    let mut sz = axis.z * ang;
    let lim = |v: &mut f32, a: usize| {
        if s.lock[a] {
            *v = 0.0;
        } else if s.use_limit[a] {
            *v = v.clamp(s.min[a].min(s.max[a]), s.max[a].max(s.min[a]));
        }
    };
    lim(&mut sx, 0);
    lim(&mut twist, 1);
    lim(&mut sz, 2);
    let sv = Vec3::new(sx, 0.0, sz);
    let swing = if sv.length() > 1e-12 { Quat::from_axis_angle(sv, sv.length()) } else { Quat::IDENTITY };
    (swing * Quat::from_axis_angle(Vec3::Y, twist)).normalize()
}

impl Scene {
    /// Recursion limit for evaluation that follows references between
    /// objects (parents, constraint targets, modifiers, bone parents).
    pub const MAX_EVAL_DEPTH: u32 = 256;
    /// Depth one reference to another object (a target, a bone parent, a
    /// modifier object) costs; plain parenting costs 1.
    pub const REF_DEPTH: u32 = 16;

    /// Pose-space matrices of every bone of armature object `idx` (Blender's
    /// `pchan->pose_mat`): the object's pose with constraints and IK. Empty
    /// for objects that are not armatures.
    pub fn pose_matrices(&self, idx: usize) -> Vec<Mat4> {
        self.pose_matrices_depth(idx, 0)
    }

    pub(crate) fn pose_matrices_depth(&self, idx: usize, depth: u32) -> Vec<Mat4> {
        let Some(o) = self.objects.get(idx) else { return Vec::new() };
        let ObjectData::Armature(ai) = o.data else { return Vec::new() };
        let Some(arm) = self.armatures.get(ai) else { return Vec::new() };
        if arm.rest_position {
            return arm.rest_matrices();
        }
        if depth >= Self::MAX_EVAL_DEPTH {
            return crate::armature::simple_pose(arm, &o.pose);
        }
        let world = self.world_matrix_depth(idx, depth + 1);
        let mut s = PoseSolver { scene: self, obj: idx, arm, world, depth: depth + 1, poses: arm.rest_matrices(), ik: vec![None; arm.bones.len()] };
        s.run();
        s.poses
    }

    /// Deform matrices (`pose_mat × arm_mat⁻¹`) of every bone.
    pub fn deform_matrices(&self, idx: usize) -> Vec<Mat4> {
        let ObjectData::Armature(ai) = self.objects[idx].data else { return Vec::new() };
        let rest = self.armatures[ai].rest_matrices();
        self.pose_matrices(idx).iter().zip(&rest).map(|(p, r)| *p * r.inverse()).collect()
    }

    /// The object's matrix from parenting alone (no constraints of its own).
    pub(crate) fn local_world_matrix(&self, idx: usize, depth: u32) -> Mat4 {
        let local = self.objects[idx].transform.matrix();
        match self.parent_matrix_depth(idx, depth) {
            Some(p) => p * local,
            None => local,
        }
    }

    /// The world matrix the object's parent contributes (Blender's parent
    /// matrix: the parent's world matrix, through the parent bone for bone
    /// parenting). `None` without a parent.
    pub(crate) fn parent_matrix_depth(&self, idx: usize, depth: u32) -> Option<Mat4> {
        let o = &self.objects[idx];
        let p = o.parent?;
        if p >= self.objects.len() || p == idx || depth >= Self::MAX_EVAL_DEPTH {
            return None;
        }
        let pw = self.world_matrix_depth(p, depth + 1);
        if o.parent_type != ParentType::Bone {
            return Some(pw);
        }
        let ObjectData::Armature(ai) = self.objects[p].data else { return Some(pw) };
        let Some(arm) = self.armatures.get(ai) else { return Some(pw) };
        let Some(b) = arm.find(&o.parent_bone) else { return Some(pw) };
        let poses = self.pose_matrices_depth(p, depth + Self::REF_DEPTH);
        let pose = poses.get(b).copied().unwrap_or(Mat4::IDENTITY);
        let bm = if arm.bones[b].use_relative_parent {
            pose * arm.bones[b].rest_matrix().inverse()
        } else {
            let mut m = pose;
            let t = m.translation_part() + m.col3(1) * arm.bones[b].length();
            m.set_col3(3, t);
            m
        };
        Some(pw * bm)
    }

    /// World matrix with parenting and object constraints, limited to
    /// `depth` levels of references.
    pub(crate) fn world_matrix_depth(&self, idx: usize, depth: u32) -> Mat4 {
        let m = self.base_world_matrix_depth(idx, depth);
        if self.objects[idx].physics.rigid_body.is_some() {
            if let Some(r) = crate::physics::rigid::override_matrix(self, idx, &m) {
                return r;
            }
        }
        m
    }

    /// World matrix from parenting and constraints only, ignoring the rigid
    /// body simulation (where a simulation starts from).
    pub fn base_world_matrix(&self, idx: usize) -> Mat4 {
        self.base_world_matrix_depth(idx, 0)
    }

    fn base_world_matrix_depth(&self, idx: usize, depth: u32) -> Mat4 {
        let o = &self.objects[idx];
        let parent = self.parent_matrix_depth(idx, depth);
        let local = o.transform.matrix();
        let mut m = match parent {
            Some(p) => p * local,
            None => local,
        };
        if !o.constraints.is_empty() && depth < Self::MAX_EVAL_DEPTH {
            let info = SpaceInfo { parent, basis: local, ..SpaceInfo::WORLD };
            let owner = Owner { object: idx, bone: None, info, rot_order: EulerOrder::XYZ };
            solve(&o.constraints, &mut m, &owner, &Ctx { scene: self, depth });
        }
        m
    }

    /// Child Of "Set Inverse": store the inverse of the constraint's current
    /// parent matrix so the owner stays where it is. `bone` names a pose bone
    /// of an armature owner. Returns false when there is no such Child Of.
    pub fn child_of_set_inverse(&mut self, idx: usize, bone: Option<&str>, constraint: usize) -> bool {
        let get = |s: &Scene| -> Option<(Target, Space)> {
            let o = &s.objects[idx];
            let list = match bone {
                Some(b) => &crate::armature::pose_bone(&o.pose, b)?.constraints,
                None => &o.constraints,
            };
            match &list.get(constraint)?.kind {
                ConstraintKind::ChildOf { target, .. } => Some((target.clone(), list[constraint].target_space)),
                _ => None,
            }
        };
        let Some((t, space)) = get(self) else { return false };
        let owner = Owner { object: idx, bone: None, info: SpaceInfo::WORLD, rot_order: EulerOrder::XYZ };
        let poses = bone.map(|_| self.pose_matrices(idx));
        let bi = bone.and_then(|b| match self.objects[idx].data {
            ObjectData::Armature(ai) => self.armatures.get(ai)?.find(b),
            _ => None,
        });
        let owner = match (&poses, bi) {
            (Some(p), Some(b)) => Owner { bone: Some((b, p.as_slice())), ..owner },
            _ => owner,
        };
        let ctx = Ctx { scene: self, depth: 0 };
        let Some(r) = ctx.target(&t, space, &owner) else { return false };
        let mut inv = r.m.inverse();
        if bone.is_some() {
            inv = inv * self.world_matrix(idx);
        }
        let o = &mut self.objects[idx];
        let list = match bone {
            Some(b) => match o.pose.iter_mut().find(|p| p.name == b) {
                Some(p) => &mut p.constraints,
                None => return false,
            },
            None => &mut o.constraints,
        };
        if let Some(ConstraintKind::ChildOf { inverse, .. }) = list.get_mut(constraint).map(|c| &mut c.kind) {
            *inverse = inv;
            return true;
        }
        false
    }
}
