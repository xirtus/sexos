// SPDX-License-Identifier: GPL-3.0-only
//! G / R / S (and Ctrl+R roll) on bones and lattice points: what the
//! viewport's transform moves in armature Edit Mode, Pose Mode and lattice
//! Edit Mode. The transform computes a world-space affine map; here it moves
//! joints (keeping connected children attached and turning roll with
//! rotations), lattice points, or pose channels (only the topmost selected
//! bones, so children follow; Auto IK swings the chain instead).
use super::{armedit, pose_bone, pose_bone_mut, BoneSel};
use crate::doc::Doc;
use sex_3d::armature::{simple_pose, Bone, PoseBone};
use sex_3d::math::Mat3;
use sex_3d::{Mat4, Quat, Vec3};

#[derive(Clone, Debug)]
pub struct EditBones {
    oi: usize,
    ai: usize,
    w: Mat4,
    inv: Mat4,
    orig: Vec<Bone>,
    /// (bone, head moves, tail moves)
    moves: Vec<(usize, bool, bool)>,
}

#[derive(Clone, Debug)]
pub struct LatticePts {
    li: usize,
    w: Mat4,
    inv: Mat4,
    orig: Vec<Vec3>,
    sel: Vec<usize>,
}

#[derive(Clone, Debug)]
pub struct PoseBones {
    oi: usize,
    obj_inv: Mat4,
    /// (bone, name, world matrix at start, connected)
    bones: Vec<(usize, String, Mat4, bool)>,
    orig: Vec<PoseBone>,
    /// Auto IK: the grabbed bone's tail in world space at the start.
    ik_tip: Option<Vec3>,
}

#[derive(Clone, Debug)]
pub enum Targets {
    Bones(EditBones),
    Lattice(LatticePts),
    Pose(PoseBones),
}

/// Targets for the current mode and the world points they move (for the
/// pivot), or None when nothing is selected.
pub fn start(d: &Doc) -> Option<(Targets, Vec<Vec3>)> {
    if let Some((oi, ai)) = super::edit_armature(d) {
        let name = &d.scene.objects[oi].name;
        let ui = d.rig.arm(name);
        let a = &d.scene.armatures[ai];
        let w = d.scene.world_matrix(oi);
        let mut moves = Vec::new();
        let mut pts = Vec::new();
        for (i, b) in a.bones.iter().enumerate() {
            let s: BoneSel = ui.get(&b.name);
            let parent_tail = a.parent(i).filter(|_| b.use_connect).is_some_and(|p| ui.get(&a.bones[p].name).tail);
            let (h, t) = (s.head || parent_tail, s.tail);
            if h || t {
                moves.push((i, h, t));
                if h {
                    pts.push(w.transform_point(b.head));
                }
                if t {
                    pts.push(w.transform_point(b.tail));
                }
            }
        }
        if moves.is_empty() {
            return None;
        }
        return Some((Targets::Bones(EditBones { oi, ai, w, inv: w.inverse(), orig: a.bones.clone(), moves }), pts));
    }
    if let Some((oi, li)) = super::edit_lattice(d) {
        let name = &d.scene.objects[oi].name;
        let sel: Vec<usize> = d.rig.lattice_sel.get(name).map(|s| s.iter().copied().filter(|&i| i < d.scene.lattices[li].points.len()).collect()).unwrap_or_default();
        if sel.is_empty() {
            return None;
        }
        let w = d.scene.world_matrix(oi);
        let orig = d.scene.lattices[li].points.clone();
        let pts = sel.iter().map(|&i| w.transform_point(orig[i])).collect();
        return Some((Targets::Lattice(LatticePts { li, w, inv: w.inverse(), orig, sel }), pts));
    }
    if let Some(oi) = super::pose_object(d) {
        let a = d.scene.armature_of(oi)?;
        let sel = super::selected_pose_bones(d, oi);
        let idx: Vec<usize> = sel.iter().filter_map(|n| a.find(n)).collect();
        // Only the topmost selected bones: children follow their parents.
        let top: Vec<usize> = idx.iter().copied().filter(|&i| !idx.iter().any(|&j| j != i && a.is_ancestor(j, i))).collect();
        if top.is_empty() {
            return None;
        }
        let ow = d.scene.world_matrix(oi);
        let poses = d.scene.pose_matrices(oi);
        let bones: Vec<(usize, String, Mat4, bool)> = top.iter().map(|&i| (i, a.bones[i].name.clone(), ow * poses[i], a.bones[i].use_connect && a.parent(i).is_some())).collect();
        let pts = bones.iter().map(|b| b.2.translation_part()).collect();
        let ik_tip = (d.rig.auto_ik && bones.len() == 1).then(|| ow.transform_point(poses[bones[0].0].transform_point(Vec3::new(0.0, a.bones[bones[0].0].length(), 0.0))));
        let orig = d.scene.objects[oi].pose.clone();
        return Some((Targets::Pose(PoseBones { oi, obj_inv: ow.inverse(), bones, orig, ik_tip }), pts));
    }
    None
}

impl Targets {
    pub fn restore(&self, d: &mut Doc) {
        match self {
            Targets::Bones(t) => d.scene.armatures[t.ai].bones.clone_from(&t.orig),
            Targets::Lattice(t) => d.scene.lattices[t.li].points.clone_from(&t.orig),
            Targets::Pose(t) => d.scene.objects[t.oi].pose.clone_from(&t.orig),
        }
        d.touch();
    }

    /// Move everything by the world map `m(pivot)`; `pivot_of` picks each
    /// target's pivot (its own origin for Individual Origins). `translate`
    /// tells Auto IK it is a move.
    pub fn apply(&self, d: &mut Doc, m: &dyn Fn(Vec3) -> Mat4, individual: bool, translate: bool) {
        match self {
            Targets::Bones(t) => {
                let bones = &mut d.scene.armatures[t.ai].bones;
                for &(i, h, tl) in &t.moves {
                    let o = &t.orig[i];
                    let wh = t.w.transform_point(o.head);
                    let wt = t.w.transform_point(o.tail);
                    let mm = m(if individual { wh } else { Vec3::ZERO });
                    let nh = if h { t.inv.transform_point(mm.transform_point(wh)) } else { o.head };
                    let nt = if tl { t.inv.transform_point(mm.transform_point(wt)) } else { o.tail };
                    bones[i].head = nh;
                    bones[i].tail = nt;
                    if h && tl {
                        // Turn the roll with the bone.
                        let lin = t.inv.mat3() * mm.mat3() * t.w.mat3();
                        let r = lin.normalized();
                        bones[i].roll = armedit::rotated_roll(o.head, o.tail, o.roll, &r, nh, nt);
                    }
                }
            }
            Targets::Lattice(t) => {
                let pts = &mut d.scene.lattices[t.li].points;
                for &i in &t.sel {
                    let wp = t.w.transform_point(t.orig[i]);
                    let mm = m(if individual { wp } else { Vec3::ZERO });
                    pts[i] = t.inv.transform_point(mm.transform_point(wp));
                }
            }
            Targets::Pose(t) => {
                d.scene.objects[t.oi].pose.clone_from(&t.orig);
                if let (Some(tip), true) = (t.ik_tip, translate) {
                    let target = m(Vec3::ZERO).transform_point(tip);
                    auto_ik(d, t.oi, t.bones[0].0, t.obj_inv.transform_point(target));
                } else {
                    let arm = d.scene.armature_of(t.oi).cloned().unwrap_or_default();
                    for (i, name, wm, connected) in &t.bones {
                        let mm = m(if individual { wm.translation_part() } else { Vec3::ZERO });
                        let pm = t.obj_inv * mm * *wm;
                        let poses = d.scene.pose_matrices(t.oi);
                        let parent = arm.parent(*i).and_then(|p| poses.get(p));
                        let chan = arm.parent_transform(*i, parent).inverse().apply(&pm);
                        let keep = pose_bone(d, t.oi, name);
                        let pb = pose_bone_mut(d, t.oi, name);
                        pb.set_from_matrix(&chan);
                        if *connected {
                            pb.location = keep.location;
                        }
                        if translate {
                            // A move keeps rotation and scale exactly.
                            pb.rotation_quaternion = keep.rotation_quaternion;
                            pb.rotation_euler = keep.rotation_euler;
                            pb.rotation_axis_angle = keep.rotation_axis_angle;
                            pb.scale = keep.scale;
                        }
                    }
                }
            }
        }
        d.touch();
    }

    /// Ctrl+R in armature Edit Mode: add `angle` to the selected bones' roll.
    pub fn roll(&self, d: &mut Doc, angle: f32) {
        if let Targets::Bones(t) = self {
            for &(i, h, tl) in &t.moves {
                if h && tl {
                    d.scene.armatures[t.ai].bones[i].roll = t.orig[i].roll + angle;
                }
            }
            d.touch();
        }
    }

    pub fn is_bones(&self) -> bool {
        matches!(self, Targets::Bones(_))
    }

    pub fn object(&self) -> usize {
        match self {
            Targets::Bones(t) => t.oi,
            Targets::Lattice(_) => usize::MAX,
            Targets::Pose(t) => t.oi,
        }
    }
}

/// Auto IK: swing the connected chain above `tip_bone` (cyclic coordinate
/// descent) so its tail reaches `target` (armature space).
pub fn auto_ik(d: &mut Doc, oi: usize, tip_bone: usize, target: Vec3) {
    let Some(arm) = d.scene.armature_of(oi).cloned() else { return };
    let mut chain = vec![tip_bone];
    let mut c = tip_bone;
    while let Some(p) = arm.parent(c) {
        chain.push(p);
        if !arm.bones[c].use_connect {
            break;
        }
        c = p;
    }
    let names: Vec<String> = chain.iter().map(|&i| arm.bones[i].name.clone()).collect();
    for _ in 0..12 {
        for (k, &j) in chain.iter().enumerate() {
            let poses = simple_pose(&arm, &d.scene.objects[oi].pose);
            let tip = poses[tip_bone].transform_point(Vec3::new(0.0, arm.bones[tip_bone].length(), 0.0));
            if tip.distance(target) < 1e-4 {
                return;
            }
            let head = poses[j].translation_part();
            let (a, b) = ((tip - head).normalize(), (target - head).normalize());
            if a == Vec3::ZERO || b == Vec3::ZERO {
                continue;
            }
            let q = Quat::from_rotation_arc(a, b);
            let r = Mat3::from_quat(q).to_mat4();
            let pm = Mat4::translation(head) * r * Mat4::translation(-head) * poses[j];
            let parent = arm.parent(j).and_then(|p| poses.get(p));
            let chan = arm.parent_transform(j, parent).inverse().apply(&pm);
            let keep = pose_bone(d, oi, &names[k]);
            let pb = pose_bone_mut(d, oi, &names[k]);
            pb.set_from_matrix(&chan);
            pb.location = keep.location;
            pb.scale = keep.scale;
        }
    }
}
