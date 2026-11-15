// SPDX-License-Identifier: GPL-3.0-only
//! Armatures: bones, rest matrices, poses and armature deformation.
//!
//! This follows Blender's data model and math (`armature.cc`,
//! `armature_deform.cc`, `math_rotation.cc`):
//!
//! - A [`Bone`] stores its armature-space `head`, `tail` and `roll` (what
//!   Blender's edit bones and `Bone.arm_head/arm_tail/arm_roll` hold). Its rest
//!   matrix (`arm_mat`) is [`vec_roll_to_mat3`] of `tail - head` and `roll`,
//!   placed at `head`; the bone's Y axis runs from head to tail.
//!   [`Armature::bone_local`] gives Blender's parent-relative `Bone.head`,
//!   `Bone.tail`, `Bone.roll` and `bone_mat`.
//! - A [`PoseBone`] (Blender's `bPoseChannel`) holds the animated location,
//!   rotation (quaternion, any Euler order or axis-angle) and scale.
//!   [`PoseBone::channel_matrix`] is `BKE_pchan_to_mat4`, and the pose-space
//!   matrix of a bone is its parent's pose matrix through
//!   [`BoneParentTransform`] (`BKE_bone_parent_transform_calc_from_matrices`),
//!   honouring Connected, Inherit Rotation, every Inherit Scale mode and
//!   Local Location. Constraints and IK are evaluated by
//!   [`crate::constraints`], which owns the full pose solve
//!   ([`crate::scene::Scene::pose_matrices`]).
//! - [`deform_positions`] is the Armature modifier: linear blend skinning or
//!   dual quaternion skinning ("Preserve Volume"), weights from vertex groups
//!   named after bones and/or bone envelopes (`distfactor_to_bone`).
//! - [`auto_weights`] ("With Automatic Weights") solves Blender's bone heat
//!   diffusion: `(L + H) w_j = H p_j` with the cotangent Laplacian `L`, `H`
//!   the inverse squared distance to the nearest visible bone and `p_j` 1
//!   where bone `j` is (one of) the nearest. It is solved per bone with a
//!   Jacobi-preconditioned conjugate gradient. Blender's weight limit
//!   (weights under 0.05 fade out by 0.025) is applied, then every vertex's
//!   weights are normalized to sum to 1 (Blender leaves the small residual).
//!   Vertices that see no bone (inside another part) fall back to their
//!   nearest bone.
//!
//! B-Bone segments are stored and read but deformation treats every bone as
//! one segment (TODO: `BKE_pchan_bbone_spline_setup` and `b_bone_deform`).
use crate::math::{EulerOrder, Mat3, Mat4, Quat, Vec3};
use crate::mesh::Mesh;

/// How a bone inherits its parent's scale (Blender's `inherit_scale`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InheritScale {
    #[default]
    Full,
    FixShear,
    Average,
    None,
    NoneLegacy,
    Aligned,
}

impl InheritScale {
    pub const NAMES: [(&'static str, InheritScale); 6] = [
        ("full", InheritScale::Full),
        ("fix_shear", InheritScale::FixShear),
        ("average", InheritScale::Average),
        ("none", InheritScale::None),
        ("none_legacy", InheritScale::NoneLegacy),
        ("aligned", InheritScale::Aligned),
    ];
    /// Blender's `inherit_scale_mode` numbers (`BONE_INHERIT_SCALE_*`).
    pub fn from_blender(v: i64) -> InheritScale {
        match v {
            1 => InheritScale::FixShear,
            2 => InheritScale::Average,
            3 => InheritScale::None,
            4 => InheritScale::NoneLegacy,
            5 => InheritScale::Aligned,
            _ => InheritScale::Full,
        }
    }
}

/// One bone of an armature (rest data).
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    /// Armature-space head (Blender's `arm_head`).
    pub head: Vec3,
    /// Armature-space tail (`arm_tail`).
    pub tail: Vec3,
    /// Armature-space roll in radians (`arm_roll`, the edit bone's roll).
    pub roll: f32,
    /// The head sits on the parent's tail; pose location is ignored.
    pub use_connect: bool,
    pub use_deform: bool,
    pub use_inherit_rotation: bool,
    pub inherit_scale: InheritScale,
    pub use_local_location: bool,
    /// Children parented to this bone use its root, not its tail
    /// (`BONE_RELATIVE_PARENTING`).
    pub use_relative_parent: bool,
    pub envelope_distance: f32,
    pub envelope_weight: f32,
    pub head_radius: f32,
    pub tail_radius: f32,
    /// Multiply vertex group weights by the envelope.
    pub use_envelope_multiply: bool,
    /// B-Bone segments (deformation uses one; see the module docs).
    pub bbone_segments: u32,
    /// Bone collection names (Blender 4.0+; layers before).
    pub collections: Vec<String>,
}

impl Bone {
    pub fn new(name: &str, head: Vec3, tail: Vec3) -> Bone {
        Bone {
            name: name.into(),
            parent: None,
            head,
            tail,
            roll: 0.0,
            use_connect: false,
            use_deform: true,
            use_inherit_rotation: true,
            inherit_scale: InheritScale::Full,
            use_local_location: true,
            use_relative_parent: false,
            envelope_distance: 0.25,
            envelope_weight: 1.0,
            head_radius: 0.1,
            tail_radius: 0.05,
            use_envelope_multiply: false,
            bbone_segments: 1,
            collections: Vec::new(),
        }
    }
    pub fn length(&self) -> f32 {
        (self.tail - self.head).length()
    }
    /// Rest orientation in armature space.
    pub fn rest_mat3(&self) -> Mat3 {
        vec_roll_to_mat3(self.tail - self.head, self.roll)
    }
    /// Rest matrix in armature space (Blender's `arm_mat`).
    pub fn rest_matrix(&self) -> Mat4 {
        Mat4::from_mat3(self.rest_mat3(), self.head)
    }
}

/// Armature data (Blender's `bArmature`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Armature {
    pub bones: Vec<Bone>,
    /// Bone collection names, in order.
    pub collections: Vec<String>,
}

/// A bone's data relative to its parent, as Blender stores it in `Bone`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneLocal {
    /// Head relative to the parent's tail, in the parent's rest frame.
    pub head: Vec3,
    pub tail: Vec3,
    pub roll: f32,
    /// Rest rotation relative to the parent (`bone_mat`).
    pub bone_mat: Mat3,
}

impl Armature {
    pub fn find(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// Add a bone; returns its index. The name is made unique.
    pub fn add_bone(&mut self, mut b: Bone) -> usize {
        b.name = crate::scene::unique_name(&b.name, |n| self.bones.iter().any(|x| x.name == n));
        if b.parent.is_some_and(|p| p >= self.bones.len()) {
            b.parent = None;
        }
        self.bones.push(b);
        self.bones.len() - 1
    }

    /// Valid parent of bone `i` (out-of-range parents and cycles read as none).
    pub fn parent(&self, i: usize) -> Option<usize> {
        let p = self.bones.get(i)?.parent?;
        (p < self.bones.len() && p != i).then_some(p)
    }

    /// Bone indices with every parent before its children. Bones in a parent
    /// cycle are appended at the end treated as roots.
    pub fn order(&self) -> Vec<usize> {
        let n = self.bones.len();
        let mut done = vec![false; n];
        let mut out = Vec::with_capacity(n);
        let mut progress = true;
        while progress && out.len() < n {
            progress = false;
            for i in 0..n {
                if done[i] {
                    continue;
                }
                if self.parent(i).is_none_or(|p| done[p]) {
                    done[i] = true;
                    out.push(i);
                    progress = true;
                }
            }
        }
        out.extend((0..n).filter(|&i| !done[i]));
        out
    }

    /// True when `a` is `b` or one of its ancestors.
    pub fn is_ancestor(&self, a: usize, b: usize) -> bool {
        let mut c = Some(b);
        let mut guard = 0;
        while let Some(i) = c {
            if i == a {
                return true;
            }
            c = self.parent(i);
            guard += 1;
            if guard > self.bones.len() {
                break;
            }
        }
        false
    }

    /// Rest matrices (`arm_mat`) of every bone.
    pub fn rest_matrices(&self) -> Vec<Mat4> {
        self.bones.iter().map(Bone::rest_matrix).collect()
    }

    /// Blender's parent-relative bone data (what `ED_armature_from_edit`
    /// writes into `Bone.head/tail/roll/bone_mat`).
    pub fn bone_local(&self, i: usize) -> BoneLocal {
        let b = &self.bones[i];
        let m = b.rest_mat3();
        match self.parent(i) {
            Some(p) => {
                let pb = &self.bones[p];
                let ip = pb.rest_mat3().inverse();
                let bone_mat = ip * m;
                let head = ip.mul_vec(b.head - pb.tail);
                let tail = ip.mul_vec(b.tail - pb.tail);
                BoneLocal { head, tail, roll: mat3_vec_to_roll(&bone_mat, tail - head), bone_mat }
            }
            None => BoneLocal { head: b.head, tail: b.tail, roll: b.roll, bone_mat: m },
        }
    }

    /// The offset matrix of bone `i` from its parent's pose space
    /// (`BKE_bone_offset_matrix_get`): `bone_mat` at the local head, moved
    /// along the parent's Y by the parent's length. Roots get `arm_mat`.
    pub fn offset_matrix(&self, i: usize) -> Mat4 {
        match self.parent(i) {
            Some(p) => {
                let l = self.bone_local(i);
                let mut m = Mat4::from_mat3(l.bone_mat, l.head);
                m.cols[3][1] += self.bones[p].length();
                m
            }
            None => self.bones[i].rest_matrix(),
        }
    }

    /// The parent transform of bone `i` given its parent's pose matrix.
    pub fn parent_transform(&self, i: usize, parent_pose: Option<&Mat4>) -> BoneParentTransform {
        let b = &self.bones[i];
        let offs = self.offset_matrix(i);
        match (self.parent(i), parent_pose) {
            (Some(p), Some(pp)) => BoneParentTransform::calc(b, &offs, Some((&self.bones[p].rest_matrix(), pp))),
            _ => BoneParentTransform::calc(b, &if self.parent(i).is_some() { b.rest_matrix() } else { offs }, None),
        }
    }
}

// ---------------------------------------------------------------------------
// Rest matrix math (vec_roll_to_mat3 / mat3_vec_to_roll)

/// Blender's `vec_roll_to_mat3_normalized`: the bone matrix whose Y axis is
/// the unit `nor`, rotated by `roll` about it. The base rotation takes +Y to
/// `nor` by the shortest arc, with the numerically careful branch near -Y.
pub fn vec_roll_to_mat3_normalized(nor: Vec3, roll: f32) -> Mat3 {
    const SAFE_THRESHOLD: f32 = 6.1e-3;
    const CRITICAL_THRESHOLD: f32 = 2.5e-4;
    const THRESHOLD_SQUARED: f32 = CRITICAL_THRESHOLD * CRITICAL_THRESHOLD;
    let (x, y, z) = (nor.x, nor.y, nor.z);
    let mut theta = 1.0 + y;
    let theta_alt = x * x + z * z;
    let bmat = if theta > SAFE_THRESHOLD || theta_alt > THRESHOLD_SQUARED {
        if theta <= SAFE_THRESHOLD {
            theta = theta_alt * 0.5 + theta_alt * theta_alt * 0.125;
        }
        // Columns: b[0] = (1 - x²/θ, x, -xz/θ), b[1] = (-x, y, -z), b[2] = (-xz/θ, z, 1 - z²/θ)
        // laid out as Blender's bMatrix[col][row].
        Mat3::from_cols(Vec3::new(1.0 - x * x / theta, -x, -x * z / theta), Vec3::new(x, y, z), Vec3::new(-x * z / theta, -z, 1.0 - z * z / theta))
    } else {
        Mat3::from_cols(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, -1.0, 0.0), Vec3::new(0.0, 0.0, 1.0))
    };
    Mat3::from_axis_angle(nor, roll) * bmat
}

/// Blender's `vec_roll_to_mat3`: `vec` need not be unit length.
pub fn vec_roll_to_mat3(vec: Vec3, roll: f32) -> Mat3 {
    let n = vec.normalize();
    let n = if n == Vec3::ZERO { Vec3::Y } else { n };
    vec_roll_to_mat3_normalized(n, roll)
}

/// Blender's `mat3_vec_to_roll`: the roll that, with direction `vec`,
/// reproduces `mat`'s orientation about its Y axis.
pub fn mat3_vec_to_roll(mat: &Mat3, vec: Vec3) -> f32 {
    let vecmat = vec_roll_to_mat3(vec, 0.0);
    let rollmat = vecmat.inverse() * *mat;
    // atan2f(rollmat[2][0], rollmat[2][2]) with Blender's [col][row].
    rollmat.cols[2].x.atan2(rollmat.cols[2].z)
}

// ---------------------------------------------------------------------------
// Pose channels

/// How a pose bone (or object) stores its rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RotationMode {
    Quaternion,
    Euler(EulerOrder),
    AxisAngle,
}

impl Default for RotationMode {
    fn default() -> Self {
        RotationMode::Quaternion
    }
}

impl RotationMode {
    /// Blender's `rotmode`: 0 quaternion, 1..6 Euler XYZ..ZYX, -1 axis-angle.
    pub fn from_blender(v: i64) -> RotationMode {
        match v {
            -1 => RotationMode::AxisAngle,
            1..=6 => RotationMode::Euler(EulerOrder::from_blender(v).unwrap_or_default()),
            _ => RotationMode::Quaternion,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            RotationMode::Quaternion => "QUATERNION",
            RotationMode::AxisAngle => "AXIS_ANGLE",
            RotationMode::Euler(o) => o.name(),
        }
    }
    pub fn from_name(s: &str) -> Option<RotationMode> {
        match s.to_ascii_uppercase().as_str() {
            "QUATERNION" => Some(RotationMode::Quaternion),
            "AXIS_ANGLE" | "AXISANGLE" => Some(RotationMode::AxisAngle),
            o => EulerOrder::from_name(o).map(RotationMode::Euler),
        }
    }
    /// The Euler order constraints use for this owner (XYZ for non-Euler).
    pub fn euler_order(self) -> EulerOrder {
        match self {
            RotationMode::Euler(o) => o,
            _ => EulerOrder::XYZ,
        }
    }
}

/// Per-bone inverse kinematics settings (`bPoseChannel.ikflag`, `stiffness`,
/// `ikstretch`, `limitmin/max`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IkSettings {
    pub lock: [bool; 3],
    pub stiffness: [f32; 3],
    pub stretch: f32,
    pub use_limit: [bool; 3],
    /// Radians.
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Default for IkSettings {
    fn default() -> Self {
        let pi = std::f32::consts::PI;
        IkSettings { lock: [false; 3], stiffness: [0.0; 3], stretch: 0.0, use_limit: [false; 3], min: [-pi; 3], max: [pi; 3] }
    }
}

/// One bone's pose (Blender's `bPoseChannel`), matched to its bone by name.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseBone {
    pub name: String,
    pub location: Vec3,
    pub rotation_mode: RotationMode,
    pub rotation_quaternion: Quat,
    /// Radians, indexed by axis.
    pub rotation_euler: Vec3,
    /// (angle, x, y, z) like Blender's `rotation_axis_angle`.
    pub rotation_axis_angle: [f32; 4],
    pub scale: Vec3,
    pub constraints: Vec<crate::constraints::Constraint>,
    pub ik: IkSettings,
}

impl PoseBone {
    pub fn new(name: &str) -> PoseBone {
        PoseBone {
            name: name.into(),
            location: Vec3::ZERO,
            rotation_mode: RotationMode::Quaternion,
            rotation_quaternion: Quat::IDENTITY,
            rotation_euler: Vec3::ZERO,
            rotation_axis_angle: [0.0, 0.0, 1.0, 0.0],
            scale: Vec3::ONE,
            constraints: Vec::new(),
            ik: IkSettings::default(),
        }
    }

    /// The rotation as a matrix (`BKE_pchan_rot_to_mat3`).
    pub fn rotation_mat3(&self) -> Mat3 {
        match self.rotation_mode {
            RotationMode::Quaternion => Mat3::from_quat(self.rotation_quaternion.normalize()),
            RotationMode::Euler(o) => Mat3::from_euler(self.rotation_euler.to_array(), o),
            RotationMode::AxisAngle => {
                let [a, x, y, z] = self.rotation_axis_angle;
                let axis = Vec3::new(x, y, z);
                if axis.length_squared() < 1e-20 {
                    Mat3::IDENTITY
                } else {
                    Mat3::from_axis_angle(axis, a)
                }
            }
        }
    }

    /// Set the rotation from a matrix in the current rotation mode.
    pub fn set_rotation_mat3(&mut self, m: &Mat3) {
        match self.rotation_mode {
            RotationMode::Quaternion => self.rotation_quaternion = m.to_quat(),
            RotationMode::Euler(o) => self.rotation_euler = Vec3::from_array(m.to_euler(o)),
            RotationMode::AxisAngle => {
                let (axis, angle) = m.to_quat().to_axis_angle();
                self.rotation_axis_angle = [angle, axis.x, axis.y, axis.z];
            }
        }
    }

    /// The bone-space channel matrix (`BKE_pchan_to_mat4`): rotation × scale,
    /// plus location unless the bone is connected.
    pub fn channel_matrix(&self, connected: bool) -> Mat4 {
        let r = self.rotation_mat3() * Mat3::diagonal(self.scale);
        Mat4::from_mat3(r, if connected { Vec3::ZERO } else { self.location })
    }

    /// Set location/rotation/scale from a bone-space matrix
    /// (`BKE_pchan_apply_mat4`).
    pub fn set_from_matrix(&mut self, m: &Mat4) {
        let (l, r, s) = m.to_loc_mat3_scale();
        self.location = l;
        self.scale = s;
        self.set_rotation_mat3(&r);
    }
}

/// Find the pose bone named `name`.
pub fn pose_bone<'a>(pose: &'a [PoseBone], name: &str) -> Option<&'a PoseBone> {
    pose.iter().find(|p| p.name == name)
}

// ---------------------------------------------------------------------------
// Parent transforms (BoneParentTransform)

/// Blender's `BoneParentTransform`: how a bone-space matrix becomes a
/// pose-space one under the parent (`rotscale_mat` for the 3x3 part,
/// `loc_mat` for the location, then `post_scale`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneParentTransform {
    pub rotscale: Mat4,
    pub loc: Mat4,
    pub post_scale: Vec3,
}

/// Blender's `orthogonalize_m3_stable`: make the other axes orthogonal to
/// `axis` by plane projection (keeps the determinant), optionally normalizing.
pub fn orthogonalize_stable(m: &Mat3, axis: usize, normalize: bool) -> Mat3 {
    let (a, b, c) = match axis {
        0 => (0, 1, 2),
        1 => (1, 0, 2),
        _ => (2, 0, 1),
    };
    let (mut v1, mut v2, mut v3) = (m.cols[a], m.cols[b], m.cols[c]);
    let l1 = v1.length_squared();
    if l1 > 0.0 {
        v2 -= v1 * (v2.dot(v1) / l1);
        v3 -= v1 * (v3.dot(v1) / l1);
        if normalize {
            v1 = v1 / l1.sqrt();
        }
    }
    let l2 = v2.length_squared();
    if l2 > 0.0 {
        v3 -= v2 * (v3.dot(v2) / l2);
        if normalize {
            v2 = v2 / l2.sqrt();
        }
    }
    if normalize {
        v3 = v3.normalize();
    }
    let mut cols = [Vec3::ZERO; 3];
    cols[a] = v1;
    cols[b] = v2;
    cols[c] = v3;
    Mat3 { cols }
}

/// Blender's `mat4_to_size_fix_shear`.
pub fn size_fix_shear(m: &Mat3) -> Vec3 {
    let s = m.scale();
    let vol = s.x * s.y * s.z;
    if vol != 0.0 {
        s * (m.determinant() / vol).abs().cbrt()
    } else {
        s
    }
}

impl BoneParentTransform {
    pub const IDENTITY: BoneParentTransform = BoneParentTransform { rotscale: Mat4::IDENTITY, loc: Mat4::IDENTITY, post_scale: Vec3::ONE };

    /// `BKE_bone_parent_transform_calc_from_matrices`. `offs` is the bone's
    /// offset matrix (its `arm_mat` for a root); `parent` the parent's
    /// (`arm_mat`, `pose_mat`).
    pub fn calc(bone: &Bone, offs: &Mat4, parent: Option<(&Mat4, &Mat4)>) -> BoneParentTransform {
        let mut r = BoneParentTransform::IDENTITY;
        let Some((parent_arm, parent_pose)) = parent else {
            r.rotscale = *offs;
            if !bone.use_local_location {
                r.loc = Mat4::translation(offs.translation_part());
            } else {
                r.loc = *offs;
            }
            return r;
        };
        let use_rotation = bone.use_inherit_rotation;
        let mode = bone.inherit_scale;
        let full = use_rotation && mode == InheritScale::Full;
        if full {
            r.rotscale = *parent_pose * *offs;
        } else {
            let mut tmat;
            if use_rotation {
                tmat = *parent_pose;
                match mode {
                    InheritScale::Full | InheritScale::FixShear => {}
                    InheritScale::None | InheritScale::Average => tmat = tmat.with_mat3(orthogonalize_stable(&tmat.mat3(), 1, true)),
                    InheritScale::Aligned => {
                        let o = orthogonalize_stable(&tmat.mat3(), 1, false);
                        r.post_scale = o.scale();
                        tmat = tmat.with_mat3(o.normalized());
                    }
                    InheritScale::NoneLegacy => tmat = tmat.with_mat3(tmat.mat3().normalized()),
                }
            } else {
                tmat = *parent_arm;
                match mode {
                    InheritScale::Full => tmat = tmat.rescale(parent_pose.scale_part()),
                    InheritScale::FixShear => tmat = tmat.rescale(size_fix_shear(&parent_pose.mat3())),
                    InheritScale::Aligned => r.post_scale = size_fix_shear(&parent_pose.mat3()),
                    _ => {}
                }
            }
            if mode == InheritScale::Average {
                let s = parent_pose.mat3().determinant().abs().cbrt();
                tmat = tmat.with_mat3(tmat.mat3() * s);
            }
            r.rotscale = tmat * *offs;
            if mode == InheritScale::FixShear {
                r.rotscale = r.rotscale.with_mat3(orthogonalize_stable(&r.rotscale.mat3(), 1, false));
            }
        }
        if !bone.use_local_location {
            let bone_loc = Mat4::translation(parent_pose.transform_point(offs.translation_part()));
            r.loc = bone_loc * parent_pose.mat3().to_mat4();
        } else if !full {
            r.loc = *parent_pose * *offs;
        } else {
            r.loc = r.rotscale;
        }
        r
    }

    /// `BKE_bone_parent_transform_apply`: bone space to pose space.
    pub fn apply(&self, m: &Mat4) -> Mat4 {
        let loc = m.translation_part();
        let mut out = self.rotscale * *m;
        out.set_col3(3, self.loc.transform_point(loc));
        out.rescale(self.post_scale)
    }

    /// `BKE_bone_parent_transform_invert`.
    pub fn inverse(&self) -> BoneParentTransform {
        let inv = |v: f32| if v != 0.0 { 1.0 / v } else { 0.0 };
        BoneParentTransform { rotscale: self.rotscale.inverse(), loc: self.loc.inverse(), post_scale: Vec3::new(inv(self.post_scale.x), inv(self.post_scale.y), inv(self.post_scale.z)) }
    }
}

/// Pose matrices of every bone with no constraints or IK: each pose bone's
/// channel matrix through its parent transform, parents first. Bones
/// without a pose bone are at rest. (The full solve with constraints is
/// [`crate::scene::Scene::pose_matrices`].)
pub fn simple_pose(arm: &Armature, pose: &[PoseBone]) -> Vec<Mat4> {
    let mut out = arm.rest_matrices();
    for i in arm.order() {
        let b = &arm.bones[i];
        let chan = pose_bone(pose, &b.name).map_or(Mat4::IDENTITY, |p| p.channel_matrix(b.use_connect && arm.parent(i).is_some()));
        let pp = arm.parent(i).map(|p| out[p]);
        out[i] = arm.parent_transform(i, pp.as_ref()).apply(&chan);
    }
    out
}

// ---------------------------------------------------------------------------
// Dual quaternions

/// Blender's `DualQuat`: rotation `quat`, dual part `trans` (both w, x, y,
/// z) and an optional scale matrix applied first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DualQuat {
    pub quat: [f32; 4],
    pub trans: [f32; 4],
    pub scale: Mat4,
    pub scale_weight: f32,
}

impl DualQuat {
    pub const ZERO: DualQuat = DualQuat { quat: [0.0; 4], trans: [0.0; 4], scale: Mat4 { cols: [[0.0; 4]; 4] }, scale_weight: 0.0 };

    /// `mat4_to_dquat(basemat, mat)` with `basemat` the bone's rest matrix and
    /// `mat` its deform matrix (`pose_mat * arm_mat⁻¹`).
    pub fn from_matrices(basemat: &Mat4, mat: &Mat4) -> DualQuat {
        let base_rs = *mat * *basemat;
        let scale = base_rs.scale_part();
        let dscale = scale - Vec3::ONE;
        let m3 = mat.mat3();
        let mut dq = DualQuat::ZERO;
        let r = if !is_orthonormal(&m3) || mat.determinant() < 0.0 || dscale.length_squared() > 1e-8 {
            let tmp = orthogonalize_stable(&base_rs.mat3(), 1, true);
            let base_r = Mat4::from_mat3(Mat3::from_quat(tmp.to_quat()), base_rs.translation_part());
            let base_inv = basemat.inverse();
            let r = base_r * base_inv;
            let s = base_r.inverse() * base_rs;
            dq.scale = *basemat * s * base_inv;
            dq.scale_weight = 1.0;
            r
        } else {
            *mat
        };
        let q = r.mat3().to_quat();
        dq.quat = [q.w, q.x, q.y, q.z];
        let t = r.translation_part();
        let q = dq.quat;
        dq.trans = [
            -0.5 * (t.x * q[1] + t.y * q[2] + t.z * q[3]),
            0.5 * (t.x * q[0] + t.y * q[3] - t.z * q[2]),
            0.5 * (-t.x * q[3] + t.y * q[0] + t.z * q[1]),
            0.5 * (t.x * q[2] - t.y * q[1] + t.z * q[0]),
        ];
        dq
    }

    /// `add_weighted_dq_dq`.
    pub fn add_weighted(&mut self, dq: &DualQuat, weight: f32) {
        let dot: f32 = (0..4).map(|i| dq.quat[i] * self.quat[i]).sum();
        let mut w = weight;
        let flipped = dot < 0.0;
        if flipped {
            w = -w;
        }
        for i in 0..4 {
            self.quat[i] += w * dq.quat[i];
            self.trans[i] += w * dq.trans[i];
        }
        if dq.scale_weight != 0.0 {
            let sw = if flipped { -w } else { w };
            self.scale = self.scale.add_mat(&dq.scale.scaled(sw));
            self.scale_weight += sw;
        }
    }

    /// `normalize_dq`.
    pub fn normalize(&mut self, total: f32) {
        let s = 1.0 / total;
        for i in 0..4 {
            self.quat[i] *= s;
            self.trans[i] *= s;
        }
        if self.scale_weight != 0.0 {
            let add = total - self.scale_weight;
            if add != 0.0 {
                for i in 0..4 {
                    self.scale.cols[i][i] += add;
                }
            }
            self.scale = self.scale.scaled(s);
            self.scale_weight = 1.0;
        }
    }

    /// `mul_v3m3_dq` (without the deform-matrix output).
    pub fn transform(&self, p: Vec3) -> Vec3 {
        let [w, x, y, z] = self.quat;
        let [t0, t1, t2, t3] = self.trans;
        let m = Mat3::from_cols(
            Vec3::new(w * w + x * x - y * y - z * z, 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)),
            Vec3::new(2.0 * (x * y - w * z), w * w + y * y - x * x - z * z, 2.0 * (y * z + w * x)),
            Vec3::new(2.0 * (x * z + w * y), 2.0 * (y * z - w * x), w * w + z * z - x * x - y * y),
        );
        let mut len2 = w * w + x * x + y * y + z * z;
        if len2 > 0.0 {
            len2 = 1.0 / len2;
        }
        let t = Vec3::new(2.0 * (-t0 * x + w * t1 - t2 * z + y * t3), 2.0 * (-t0 * y + t1 * z - x * t3 + w * t2), 2.0 * (-t0 * z + x * t2 + w * t3 - t1 * y));
        let mut r = p;
        if self.scale_weight != 0.0 {
            r = self.scale.transform_point(r);
        }
        (m.mul_vec(r) + t) * len2
    }
}

fn is_orthonormal(m: &Mat3) -> bool {
    const EPS: f32 = 1.5e-5;
    for i in 0..3 {
        if (m.cols[i].length_squared() - 1.0).abs() > EPS {
            return false;
        }
        for j in i + 1..3 {
            if m.cols[i].dot(m.cols[j]).abs() > EPS {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Envelopes and deformation

/// Blender's `distfactor_to_bone`: 1 inside the bone's capsule (radius
/// interpolated head to tail), falling to 0 over `rdist` outside it.
pub fn distfactor_to_bone(p: Vec3, b1: Vec3, b2: Vec3, rad1: f32, rad2: f32, rdist: f32) -> f32 {
    let bdelta = b2 - b1;
    let l = bdelta.length();
    let bdir = if l > 0.0 { bdelta / l } else { Vec3::ZERO };
    let pdelta = p - b1;
    let a = bdir.dot(pdelta);
    let hsqr = pdelta.length_squared();
    let (dist_sq, rad) = if a < 0.0 {
        ((b1 - p).length_squared(), rad1)
    } else if a > l {
        ((b2 - p).length_squared(), rad2)
    } else {
        let r = if l != 0.0 {
            let t = a / l;
            t * rad2 + (1.0 - t) * rad1
        } else {
            rad1
        };
        (hsqr - a * a, r)
    };
    if dist_sq < rad * rad {
        return 1.0;
    }
    let outer = (rad + rdist) * (rad + rdist);
    if rdist == 0.0 || dist_sq >= outer {
        return 0.0;
    }
    let a = dist_sq.sqrt() - rad;
    1.0 - (a * a) / (rdist * rdist)
}

/// Armature modifier settings that [`deform_positions`] needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeformOptions {
    pub use_vertex_groups: bool,
    pub use_bone_envelopes: bool,
    /// Dual quaternion skinning.
    pub preserve_volume: bool,
    /// Overall influence group (`armature_def_nr`).
    pub vertex_group: Option<String>,
    pub invert_vertex_group: bool,
}

/// Deform `coords` (vertex positions of `mesh`, in the mesh object's space)
/// by the armature in pose `pose` (pose-space bone matrices). `premat` takes
/// mesh space to armature space. `prev` holds the coordinates the previous
/// modifier started from (Multi Modifier): the armature then deforms those
/// and blends with `coords` by the overall group weight.
pub fn deform_positions(mesh: &Mesh, coords: &[Vec3], arm: &Armature, pose: &[Mat4], premat: &Mat4, opts: &DeformOptions, prev: Option<&[Vec3]>) -> Vec<Vec3> {
    let n = coords.len();
    let rest = arm.rest_matrices();
    let chan: Vec<Mat4> = (0..arm.bones.len()).map(|i| pose.get(i).copied().unwrap_or(rest[i]) * rest[i].inverse()).collect();
    let dqs: Vec<DualQuat> = if opts.preserve_volume { (0..arm.bones.len()).map(|i| DualQuat::from_matrices(&rest[i], &chan[i])).collect() } else { Vec::new() };
    // Vertex group -> bone (deforming bones only).
    let group_bone: Vec<Option<usize>> = mesh.vertex_groups.iter().map(|g| arm.find(g).filter(|&b| arm.bones[b].use_deform)).collect();
    let def_nr = opts.vertex_group.as_deref().filter(|g| !g.is_empty()).and_then(|g| mesh.vertex_group_index(g));
    let postmat = premat.inverse();
    let prev = prev.filter(|p| p.len() == n);
    let mut out = coords.to_vec();
    for i in 0..n {
        let mut armature_weight = 1.0;
        let mut prevco_weight = 0.0;
        if let Some(g) = def_nr {
            armature_weight = mesh.weight(i, g);
            if opts.invert_vertex_group {
                armature_weight = 1.0 - armature_weight;
            }
            if prev.is_some() {
                prevco_weight = 1.0 - armature_weight;
                armature_weight = 1.0;
            }
        }
        let source = match prev {
            Some(p) => {
                if prevco_weight == 1.0 {
                    continue;
                }
                p[i]
            }
            None => {
                if armature_weight == 0.0 {
                    continue;
                }
                coords[i]
            }
        };
        let co = premat.transform_point(source);
        let mut vec = Vec3::ZERO;
        let mut dq = DualQuat::ZERO;
        let mut contrib = 0.0f32;
        let mut accumulate = |b: usize, w: f32, vec: &mut Vec3, dq: &mut DualQuat| {
            if w == 0.0 {
                return;
            }
            if opts.preserve_volume {
                dq.add_weighted(&dqs[b], w);
            } else {
                *vec += (chan[b].transform_point(co) - co) * w;
            }
        };
        let weights = mesh.vertex_weights(i);
        let envelope = |vec: &mut Vec3, dq: &mut DualQuat, acc: &mut dyn FnMut(usize, f32, &mut Vec3, &mut DualQuat)| -> f32 {
            let mut c = 0.0;
            for (b, bone) in arm.bones.iter().enumerate() {
                if !bone.use_deform {
                    continue;
                }
                let f = distfactor_to_bone(co, bone.head, bone.tail, bone.head_radius, bone.tail_radius, bone.envelope_distance) * bone.envelope_weight;
                if f > 0.0 {
                    acc(b, f, vec, dq);
                    c += f;
                }
            }
            c
        };
        if opts.use_vertex_groups && !weights.is_empty() {
            let mut deformed = false;
            for &(g, w) in weights {
                let Some(Some(b)) = group_bone.get(g as usize).copied() else { continue };
                deformed = true;
                let bone = &arm.bones[b];
                let mut w = w;
                if bone.use_envelope_multiply {
                    w *= distfactor_to_bone(co, bone.head, bone.tail, bone.head_radius, bone.tail_radius, bone.envelope_distance);
                }
                if w != 0.0 {
                    accumulate(b, w, &mut vec, &mut dq);
                    contrib += w;
                }
            }
            if !deformed && opts.use_bone_envelopes {
                contrib += envelope(&mut vec, &mut dq, &mut accumulate);
            }
        } else if opts.use_bone_envelopes {
            contrib += envelope(&mut vec, &mut dq, &mut accumulate);
        }
        let mut co_out = co;
        if contrib > 0.0001 {
            if opts.preserve_volume {
                dq.normalize(contrib);
                if armature_weight != 1.0 {
                    let d = dq.transform(co) - co;
                    co_out = co + d * armature_weight;
                } else {
                    co_out = dq.transform(co);
                }
            } else {
                co_out = co + vec * (armature_weight / contrib);
            }
        }
        let mut res = postmat.transform_point(co_out);
        if prev.is_some() {
            let mw = 1.0 - prevco_weight;
            res = coords[i] * prevco_weight + res * mw;
        }
        out[i] = res;
    }
    out
}

// ---------------------------------------------------------------------------
// Weight generation ("Parent with Automatic / Envelope Weights / Empty Groups")

/// Closest point on segment `a`-`b` to `p`.
fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let d = b - a;
    let l2 = d.length_squared();
    if l2 <= 0.0 {
        return a;
    }
    a + d * ((p - a).dot(d) / l2).clamp(0.0, 1.0)
}

/// Deforming bones as (index, head, tail) in mesh space.
fn deform_segments(arm: &Armature, arm_to_mesh: &Mat4) -> Vec<(usize, Vec3, Vec3)> {
    arm.bones.iter().enumerate().filter(|(_, b)| b.use_deform).map(|(i, b)| (i, arm_to_mesh.transform_point(b.head), arm_to_mesh.transform_point(b.tail))).collect()
}

/// "With Empty Groups": a vertex group per deforming bone, no weights.
pub fn empty_groups(mesh: &mut Mesh, arm: &Armature) {
    for b in arm.bones.iter().filter(|b| b.use_deform) {
        mesh.ensure_vertex_group(&b.name);
    }
}

/// "With Envelope Weights": each deforming bone's group gets the envelope
/// factor ([`distfactor_to_bone`]) of every vertex inside its envelope.
/// `arm_to_mesh` takes armature space into mesh space.
pub fn envelope_weights(mesh: &mut Mesh, arm: &Armature, arm_to_mesh: &Mat4) {
    let mesh_to_arm = arm_to_mesh.inverse();
    for (b, bone) in arm.bones.iter().enumerate() {
        if !bone.use_deform {
            continue;
        }
        let g = mesh.ensure_vertex_group(&arm.bones[b].name);
        for v in 0..mesh.positions.len() {
            let p = mesh_to_arm.transform_point(mesh.positions[v]);
            let f = distfactor_to_bone(p, bone.head, bone.tail, bone.head_radius, bone.tail_radius, bone.envelope_distance);
            if f > 0.0 {
                mesh.set_weight(v, g, f);
            } else {
                mesh.remove_weight(v, g);
            }
        }
    }
}

/// Symmetric sparse matrix in compressed rows, for the heat solve.
struct Sparse {
    start: Vec<usize>,
    col: Vec<usize>,
    val: Vec<f64>,
    diag: Vec<f64>,
}

impl Sparse {
    fn mul(&self, x: &[f64], y: &mut [f64]) {
        for i in 0..self.diag.len() {
            let mut s = self.diag[i] * x[i];
            for k in self.start[i]..self.start[i + 1] {
                s += self.val[k] * x[self.col[k]];
            }
            y[i] = s;
        }
    }

    /// Jacobi-preconditioned conjugate gradient from `x`.
    fn solve(&self, b: &[f64], x: &mut [f64], iters: usize, tol: f64) {
        let n = b.len();
        let mut r = vec![0.0; n];
        self.mul(x, &mut r);
        for i in 0..n {
            r[i] = b[i] - r[i];
        }
        let inv: Vec<f64> = self.diag.iter().map(|&d| if d.abs() > 1e-300 { 1.0 / d } else { 1.0 }).collect();
        let mut z: Vec<f64> = (0..n).map(|i| r[i] * inv[i]).collect();
        let mut p = z.clone();
        let mut rz: f64 = (0..n).map(|i| r[i] * z[i]).sum();
        let bn: f64 = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-300);
        let mut ap = vec![0.0; n];
        for _ in 0..iters {
            let rn: f64 = r.iter().map(|v| v * v).sum::<f64>().sqrt();
            if rn / bn < tol {
                break;
            }
            self.mul(&p, &mut ap);
            let pap: f64 = (0..n).map(|i| p[i] * ap[i]).sum();
            if pap.abs() < 1e-300 {
                break;
            }
            let alpha = rz / pap;
            for i in 0..n {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
                z[i] = r[i] * inv[i];
            }
            let rz2: f64 = (0..n).map(|i| r[i] * z[i]).sum();
            let beta = rz2 / rz.max(1e-300);
            rz = rz2;
            for i in 0..n {
                p[i] = z[i] + beta * p[i];
            }
        }
    }
}

/// "With Automatic Weights": bone heat weighting (see the module docs).
/// Creates or replaces a vertex group per deforming bone. `arm_to_mesh`
/// takes armature space into mesh space (rest positions are used).
pub fn auto_weights(mesh: &mut Mesh, arm: &Armature, arm_to_mesh: &Mat4) {
    const WEIGHT_LIMIT_START: f64 = 0.05;
    const WEIGHT_LIMIT_END: f64 = 0.025;
    const DISTANCE_EPSILON: f32 = 1e-4;
    let segs = deform_segments(arm, arm_to_mesh);
    let n = mesh.positions.len();
    if segs.is_empty() || n == 0 {
        empty_groups(mesh, arm);
        return;
    }
    let pos = mesh.positions.clone();
    // Cotangent Laplacian and vertex areas.
    let tris = mesh.triangulate();
    let mut area = vec![0.0f64; n];
    let mut w: std::collections::HashMap<(usize, usize), f64> = std::collections::HashMap::new();
    for t in &tris {
        let v = t.verts.map(|i| i as usize);
        let p = v.map(|i| pos[i]);
        let a2 = (p[1] - p[0]).cross(p[2] - p[0]).length() as f64;
        for c in 0..3 {
            area[v[c]] += a2 / 6.0;
            // Angle at corner c is opposite edge (c+1, c+2).
            let (e1, e2) = (p[(c + 1) % 3] - p[c], p[(c + 2) % 3] - p[c]);
            let cr = e1.cross(e2).length() as f64;
            let cot = if cr > 1e-12 { e1.dot(e2) as f64 / cr } else { 0.0 };
            let (i, j) = (v[(c + 1) % 3], v[(c + 2) % 3]);
            let k = if i < j { (i, j) } else { (j, i) };
            *w.entry(k).or_insert(0.0) += 0.5 * cot.max(1e-6);
        }
    }
    for e in &mesh.loose_edges {
        let (i, j) = (e[0] as usize, e[1] as usize);
        if i < n && j < n && i != j {
            let k = if i < j { (i, j) } else { (j, i) };
            w.entry(k).or_insert(1e-3);
        }
    }
    // Heat sources: nearest visible bones per vertex.
    let bvh = crate::modifiers::bvh::TriBvh::new(mesh);
    let scale = mesh.bounds().size().length().max(1e-6);
    let mut closest: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut h = vec![0.0f64; n];
    let mut nearest_any = vec![0usize; n];
    for v in 0..n {
        let d: Vec<f32> = segs.iter().map(|s| (closest_on_segment(pos[v], s.1, s.2) - pos[v]).length()).collect();
        let mind = d.iter().copied().fold(f32::INFINITY, f32::min);
        nearest_any[v] = d.iter().position(|&x| x == mind).unwrap_or(0);
        for (j, s) in segs.iter().enumerate() {
            if d[j] > mind * (1.0 + DISTANCE_EPSILON) + 1e-9 {
                continue;
            }
            // Visible: no surface between the vertex and the bone.
            let q = closest_on_segment(pos[v], s.1, s.2);
            let dir = q - pos[v];
            let len = dir.length();
            let visible = len < 1e-6 * scale || {
                let dn = dir / len;
                let o = pos[v] + dn * (1e-4 * scale);
                bvh.ray(o, dn, (len - 2e-4 * scale).max(0.0)).is_none()
            };
            if visible {
                closest[v].push(j);
            }
        }
        if !closest[v].is_empty() {
            let md = (mind as f64).max(1e-4);
            h[v] = closest[v].len() as f64 / (md * md);
        }
    }
    // Symmetric system (L + 2A·H): Blender's rows are scaled by 1/(2A).
    let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut diag = vec![0.0f64; n];
    for (&(i, j), &x) in &w {
        rows[i].push((j, -x));
        rows[j].push((i, -x));
        diag[i] += x;
        diag[j] += x;
    }
    let hs: Vec<f64> = (0..n).map(|v| h[v] * 2.0 * area[v].max(1e-12)).collect();
    for v in 0..n {
        diag[v] += hs[v] + 1e-10;
    }
    let mut start = vec![0usize; n + 1];
    let mut col = Vec::new();
    let mut val = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        for &(j, x) in r {
            col.push(j);
            val.push(x);
        }
        start[i + 1] = col.len();
    }
    let a = Sparse { start, col, val, diag };
    let mut result: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    for (j, _) in segs.iter().enumerate() {
        let b: Vec<f64> = (0..n).map(|v| if closest[v].contains(&j) { hs[v] / closest[v].len() as f64 } else { 0.0 }).collect();
        if b.iter().all(|&x| x == 0.0) {
            continue;
        }
        let mut x = vec![0.0f64; n];
        a.solve(&b, &mut x, (4 * n).clamp(200, 4000), 1e-7);
        for v in 0..n {
            let s = x[v];
            let wv = if s > WEIGHT_LIMIT_START {
                s
            } else if s > WEIGHT_LIMIT_END {
                (s - WEIGHT_LIMIT_END) / (WEIGHT_LIMIT_START - WEIGHT_LIMIT_END) * WEIGHT_LIMIT_START
            } else {
                0.0
            };
            if wv > 0.0 && wv.is_finite() {
                result[v].push((j, wv as f32));
            }
        }
    }
    let groups: Vec<usize> = segs.iter().map(|s| mesh.ensure_vertex_group(&arm.bones[s.0].name)).collect();
    for v in 0..n {
        for &g in &groups {
            mesh.remove_weight(v, g);
        }
        let mut r = std::mem::take(&mut result[v]);
        let sum: f32 = r.iter().map(|e| e.1).sum();
        if sum <= 1e-6 {
            r = vec![(nearest_any[v], 1.0)];
        } else {
            r.iter_mut().for_each(|e| e.1 /= sum);
        }
        for (j, x) in r {
            mesh.set_weight(v, groups[j], x);
        }
    }
}
