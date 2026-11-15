// SPDX-License-Identifier: GPL-3.0-only
//! glTF skins: joints to armature bones and back.
//!
//! Import groups skins that share joints into one armature. Bones rest at
//! each joint's bind pose (the inverse of its inverse bind matrix, or the
//! joint node's own transform without one) and point along the joint's +Y;
//! a bone reaches its first child's head, a leaf takes its parent's length.
//! Like Blender's exporter, a joint's frame is its bone's frame: the axis
//! change applies once, on the left (`C J`), so bones run along each joint's
//! local +Y and joint nodes below the root carry plain bone-relative
//! transforms.
//! The joint nodes' current transforms become the pose. When every root
//! joint hangs under the same plain node, that node becomes the armature
//! object; otherwise a new armature object sits at the scene origin.
//!
//! Export writes one node per bone under the armature object's node (with
//! the current pose), and one skin per skinned mesh object: joints are all
//! of the armature's bones, inverse bind matrices their rest matrices, and
//! the mesh vertices are written in armature space.
use super::q_c;
use crate::armature::{mat3_vec_to_roll, Armature, Bone, PoseBone};
use crate::math::{Mat4, Vec3};
use crate::mesh::Mesh;
use std::collections::HashMap;

/// `C`, the +90° X turn from Y-up to Z-up, as a matrix.
pub(super) fn c_mat() -> Mat4 {
    Mat4::from_trs(Vec3::ZERO, q_c(), Vec3::ONE)
}

/// One armature built from one or more skins.
pub(super) struct Group {
    pub name: String,
    /// The node that becomes the armature object, if any.
    pub host: Option<usize>,
    /// The first joint in node order (where a new armature object goes).
    pub first_joint: usize,
    pub armature: Armature,
    pub pose: Vec<PoseBone>,
}

/// Every skin's armature: `node_bone[n]` is (group, bone) for joint nodes,
/// `skin_group[s]` the group of skin `s`.
pub(super) struct Skins {
    pub groups: Vec<Group>,
    pub node_bone: Vec<Option<(usize, usize)>>,
    pub skin_group: Vec<usize>,
}

/// Inputs about the node tree for [`build`].
pub(super) struct NodeInfo<'a> {
    pub names: Vec<Option<&'a str>>,
    pub included: &'a [bool],
    pub parent: &'a [Option<usize>],
    /// glTF world matrices (Y-up) at the file's current pose.
    pub world: Vec<Mat4>,
    /// Nodes carrying a mesh, camera or light.
    pub has_data: Vec<bool>,
}

/// Group the skins into armatures and build their bones and poses.
/// `skins[s]` is (name, joints, inverse bind matrices).
pub(super) fn build(info: &NodeInfo, skins: &[(Option<String>, Vec<usize>, Option<Vec<Mat4>>)]) -> Skins {
    let n = info.included.len();
    // Union skins that share joints.
    let mut node_group: Vec<Option<usize>> = vec![None; n];
    let mut skin_group = vec![usize::MAX; skins.len()];
    let mut ngroups = 0;
    for (si, (_, joints, _)) in skins.iter().enumerate() {
        let joints: Vec<usize> = joints.iter().copied().filter(|&j| j < n && info.included[j]).collect();
        let found: Vec<usize> = joints.iter().filter_map(|&j| node_group[j]).collect();
        let g = found.first().copied().unwrap_or_else(|| {
            ngroups += 1;
            ngroups - 1
        });
        for x in node_group.iter_mut().flatten().chain(skin_group[..si].iter_mut()) {
            if found.contains(x) {
                *x = g;
            }
        }
        for &j in &joints {
            node_group[j] = Some(g);
        }
        skin_group[si] = g;
    }

    // Each joint's bind matrix in glTF axes, from the first skin listing it.
    let mut bind: HashMap<usize, Mat4> = HashMap::new();
    for (_, joints, ibm) in skins {
        for (k, &j) in joints.iter().enumerate() {
            if j < n && info.included[j] && !bind.contains_key(&j) {
                if let Some(m) = ibm.as_ref().and_then(|v| v.get(k)) {
                    bind.insert(j, m.inverse());
                }
            }
        }
    }

    let mut node_bone = vec![None; n];
    let mut groups = Vec::new();
    for g in 0..ngroups {
        let members: Vec<usize> = (0..n).filter(|&j| node_group[j] == Some(g)).collect();
        let Some(&first_joint) = members.first() else {
            // Emptied by a merge: keep the index valid.
            groups.push(Group { name: String::new(), host: None, first_joint: usize::MAX, armature: Armature::default(), pose: Vec::new() });
            continue;
        };
        // Nearest ancestor joint of the same group.
        let parent_joint = |j: usize| {
            let mut c = info.parent[j];
            for _ in 0..n {
                match c {
                    Some(p) if node_group[p] == Some(g) => return Some(p),
                    Some(p) => c = info.parent[p],
                    None => return None,
                }
            }
            None
        };
        // The host: the one plain parent of every root joint.
        let roots: Vec<usize> = members.iter().copied().filter(|&j| parent_joint(j).is_none()).collect();
        let host = roots.first().and_then(|&r| info.parent[r]).filter(|&p| {
            node_group[p].is_none() && !info.has_data[p] && roots.iter().all(|&r| info.parent[r] == Some(p)) && !groups.iter().any(|x: &Group| x.host == Some(p))
        });
        let arm_inv = host.map_or(Mat4::IDENTITY, |h| info.world[h].inverse());
        // Parents first, in node order otherwise.
        let mut order = Vec::with_capacity(members.len());
        let mut done = vec![false; n];
        while order.len() < members.len() {
            let before = order.len();
            for &j in &members {
                if !done[j] && parent_joint(j).is_none_or(|p| done[p]) {
                    done[j] = true;
                    order.push(j);
                }
            }
            if order.len() == before {
                break;
            }
        }
        let pose_of = |j: usize| c_mat() * arm_inv * info.world[j];
        let rest_of = |j: usize| bind.get(&j).map_or_else(|| pose_of(j), |b| c_mat() * *b);
        let heads: HashMap<usize, Vec3> = order.iter().map(|&j| (j, rest_of(j).translation_part())).collect();
        let mut arm = Armature::default();
        let mut lengths: HashMap<usize, f32> = HashMap::new();
        for &j in &order {
            let (head, r, _) = rest_of(j).to_loc_mat3_scale();
            let child = order.iter().filter(|&&c| parent_joint(c) == Some(j)).map(|c| heads[c].distance(head)).find(|&d| d > 1e-4);
            let len = child.or_else(|| parent_joint(j).and_then(|p| lengths.get(&p).copied())).unwrap_or(1.0);
            lengths.insert(j, len);
            let v = r.mul_vec(Vec3::Y) * len;
            let name = info.names[j].map_or_else(|| format!("Bone{j}"), str::to_string);
            let mut b = Bone::new(&name, head, head + v);
            b.roll = mat3_vec_to_roll(&r, v);
            b.parent = parent_joint(j).and_then(|p| node_bone[p]).map(|(_, bi)| bi);
            let bi = arm.add_bone(b);
            node_bone[j] = Some((g, bi));
        }
        // The pose that puts each bone at its joint's current transform.
        let mut pose_mats: Vec<Mat4> = vec![Mat4::IDENTITY; arm.bones.len()];
        let mut pose = Vec::new();
        for &j in &order {
            let bi = node_bone[j].expect("joint bone").1;
            let target = pose_of(j);
            let parent_pose = arm.parent(bi).map(|p| pose_mats[p]);
            let chan = arm.parent_transform(bi, parent_pose.as_ref()).inverse().apply(&target);
            pose_mats[bi] = target;
            if !chan.approx_eq(&Mat4::IDENTITY, 1e-5) {
                let mut pb = PoseBone::new(&arm.bones[bi].name);
                pb.set_from_matrix(&chan);
                pose.push(pb);
            }
        }
        let name = skins.iter().zip(&skin_group).find(|(_, &sg)| sg == g).and_then(|(s, _)| s.0.clone()).unwrap_or_else(|| "Armature".into());
        groups.push(Group { name, host, first_joint, armature: arm, pose });
    }
    Skins { groups, node_bone, skin_group }
}

/// Per-vertex JOINTS_0 / WEIGHTS_0 for `mesh`: the four largest weights of
/// groups named like bones of `joints`, normalized to sum to one.
pub(super) fn vertex_joints(mesh: &Mesh, joints: &[String]) -> Vec<([u16; 4], [f32; 4])> {
    let group_joint: Vec<Option<u16>> = mesh.vertex_groups.iter().map(|g| joints.iter().position(|j| j == g).map(|j| j as u16)).collect();
    (0..mesh.positions.len())
        .map(|v| {
            let mut ws: Vec<(u16, f32)> = mesh.vertex_weights(v).iter().filter_map(|&(g, w)| Some((group_joint.get(g as usize).copied().flatten()?, w))).filter(|&(_, w)| w > 0.0).collect();
            ws.sort_by(|a, b| b.1.total_cmp(&a.1));
            ws.truncate(4);
            let sum: f32 = ws.iter().map(|w| w.1).sum();
            let (mut j, mut w) = ([0u16; 4], [0f32; 4]);
            for (k, &(jj, ww)) in ws.iter().enumerate() {
                j[k] = jj;
                w[k] = ww / sum;
            }
            (j, w)
        })
        .collect()
}
