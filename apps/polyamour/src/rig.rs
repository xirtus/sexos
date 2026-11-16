// SPDX-License-Identifier: GPL-3.0-only
//! Rigging in the editor, Blender's way: armature Edit Mode (bones as
//! heads and tails, extrude, roll, parenting), Pose Mode (posing, clearing,
//! copy and flipped paste, auto IK), parenting meshes to armatures, Weight
//! Paint mode and vertex groups, shape keys (including editing a key in Edit
//! Mode) and lattice Edit Mode.
//!
//! The rigging data itself (armatures, poses, constraints, vertex groups,
//! shape keys, lattices) lives in sex-3d's scene and is saved by its polyops
//! writer. What Blender keeps on the side (bone and lattice point selection,
//! active vertex group and shape key, display settings, the weight paint
//! brush) is [`RigState`], restored with undo and saved in `#@rig` lines.
pub mod armedit;
pub mod cons;
pub mod paint;
pub mod pose;
pub mod props;
pub mod shapes;
pub mod xform;
#[cfg(test)]
mod tests;

use crate::doc::{Doc, Mode};
use crate::ui::Op;
use sex_3d::armature::{Armature, PoseBone};
use sex_3d::scene::{ObjectData, ParentType};
use sex_3d::{Mat4, Transform, Vec3};
use std::collections::{BTreeMap, BTreeSet};

/// How bones are drawn (Armature > Viewport Display > Display As).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoneDisplay {
    #[default]
    Octahedral,
    Stick,
    BBone,
    Envelope,
    Wire,
}

impl BoneDisplay {
    pub const ALL: [BoneDisplay; 5] = [BoneDisplay::Octahedral, BoneDisplay::Stick, BoneDisplay::BBone, BoneDisplay::Envelope, BoneDisplay::Wire];
    pub fn label(self) -> &'static str {
        match self {
            BoneDisplay::Octahedral => "Octahedral",
            BoneDisplay::Stick => "Stick",
            BoneDisplay::BBone => "B-Bone",
            BoneDisplay::Envelope => "Envelope",
            BoneDisplay::Wire => "Wire",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            BoneDisplay::Octahedral => "OCTAHEDRAL",
            BoneDisplay::Stick => "STICK",
            BoneDisplay::BBone => "BBONE",
            BoneDisplay::Envelope => "ENVELOPE",
            BoneDisplay::Wire => "WIRE",
        }
    }
    pub fn from_id(s: &str) -> Option<BoneDisplay> {
        BoneDisplay::ALL.into_iter().find(|d| d.id() == s || d.label() == s)
    }
}

/// Selection of one bone: its body, head (root) and tail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoneSel {
    pub body: bool,
    pub head: bool,
    pub tail: bool,
}

impl BoneSel {
    pub const ALL: BoneSel = BoneSel { body: true, head: true, tail: true };
    pub fn any(self) -> bool {
        self.body || self.head || self.tail
    }
}

/// Per-armature-object UI state.
#[derive(Clone, Debug, PartialEq)]
pub struct ArmUi {
    pub sel: BTreeMap<String, BoneSel>,
    pub active: Option<String>,
    pub display: BoneDisplay,
    pub show_names: bool,
    pub show_axes: bool,
    pub show_in_front: bool,
    /// Bone collections hidden in the viewport.
    pub hidden_collections: BTreeSet<String>,
    /// Bones hidden (H in Pose and Edit Mode).
    pub hidden: BTreeSet<String>,
    /// The active bone collection.
    pub active_collection: usize,
}

impl Default for ArmUi {
    fn default() -> Self {
        ArmUi {
            sel: BTreeMap::new(),
            active: None,
            display: BoneDisplay::Octahedral,
            show_names: false,
            show_axes: false,
            show_in_front: false,
            hidden_collections: BTreeSet::new(),
            hidden: BTreeSet::new(),
            active_collection: 0,
        }
    }
}

impl ArmUi {
    pub fn get(&self, bone: &str) -> BoneSel {
        self.sel.get(bone).copied().unwrap_or_default()
    }
    pub fn set(&mut self, bone: &str, s: BoneSel) {
        if s.any() {
            self.sel.insert(bone.to_string(), s);
        } else {
            self.sel.remove(bone);
        }
    }
    pub fn clear(&mut self) {
        self.sel.clear();
    }
}

/// Weight Paint brushes (Blender's weight paint tools and blend modes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WpBrush {
    #[default]
    Draw,
    Blur,
    Average,
    Smear,
    Gradient,
}

impl WpBrush {
    pub const ALL: [WpBrush; 5] = [WpBrush::Draw, WpBrush::Blur, WpBrush::Average, WpBrush::Smear, WpBrush::Gradient];
    pub fn label(self) -> &'static str {
        match self {
            WpBrush::Draw => "Draw",
            WpBrush::Blur => "Blur",
            WpBrush::Average => "Average",
            WpBrush::Smear => "Smear",
            WpBrush::Gradient => "Gradient",
        }
    }
    pub fn from_label(s: &str) -> Option<WpBrush> {
        WpBrush::ALL.into_iter().find(|b| b.label().eq_ignore_ascii_case(s))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WpBlend {
    #[default]
    Mix,
    Add,
    Subtract,
}

impl WpBlend {
    pub const ALL: [WpBlend; 3] = [WpBlend::Mix, WpBlend::Add, WpBlend::Subtract];
    pub fn label(self) -> &'static str {
        match self {
            WpBlend::Mix => "Mix",
            WpBlend::Add => "Add",
            WpBlend::Subtract => "Subtract",
        }
    }
    pub fn from_label(s: &str) -> Option<WpBlend> {
        WpBlend::ALL.into_iter().find(|b| b.label().eq_ignore_ascii_case(s))
    }
}

/// The weight paint tool settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightPaint {
    pub brush: WpBrush,
    pub blend: WpBlend,
    pub weight: f32,
    pub strength: f32,
    /// Brush radius in pixels.
    pub radius: f32,
    pub auto_normalize: bool,
    pub x_mirror: bool,
    /// Weight the vertex group panel's Assign gives (Edit Mode).
    pub vg_weight: f32,
}

impl Default for WeightPaint {
    fn default() -> Self {
        WeightPaint { brush: WpBrush::Draw, blend: WpBlend::Mix, weight: 1.0, strength: 1.0, radius: 50.0, auto_normalize: false, x_mirror: false, vg_weight: 1.0 }
    }
}

/// Everything rigging keeps outside the scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RigState {
    /// Armature objects by name.
    pub arms: BTreeMap<String, ArmUi>,
    /// Active vertex group index per mesh object.
    pub active_group: BTreeMap<String, usize>,
    /// Active shape key index per mesh object.
    pub active_shape: BTreeMap<String, usize>,
    /// Shape Key Lock (pin): show only the active key.
    pub shape_lock: BTreeSet<String>,
    /// Shape key Edit Mode toggle: show the key mix in Edit Mode.
    pub shape_edit_mode: BTreeSet<String>,
    /// Locked vertex groups: (object, group).
    pub group_lock: BTreeSet<(String, String)>,
    /// Selected lattice points per lattice object.
    pub lattice_sel: BTreeMap<String, BTreeSet<usize>>,
    /// Pose Mode's Auto IK option.
    pub auto_ik: bool,
    pub wp: WeightPaint,
    /// Pose clipboard (Ctrl+C in Pose Mode).
    pub clipboard: Vec<PoseBone>,
    /// Editing shape key `key` of `object` in Edit Mode: the mesh holds the
    /// key's positions, `basis` the Basis until Edit Mode ends.
    pub shape_stash: Option<(String, usize, Vec<Vec3>)>,
}

impl RigState {
    pub fn arm(&self, object: &str) -> ArmUi {
        self.arms.get(object).cloned().unwrap_or_default()
    }
    pub fn arm_mut(&mut self, object: &str) -> &mut ArmUi {
        self.arms.entry(object.to_string()).or_default()
    }
    pub fn active_group(&self, object: &str) -> usize {
        self.active_group.get(object).copied().unwrap_or(0)
    }
    pub fn active_shape(&self, object: &str) -> usize {
        self.active_shape.get(object).copied().unwrap_or(0)
    }
    pub fn rename_object(&mut self, old: &str, new: &str) {
        if let Some(a) = self.arms.remove(old) {
            self.arms.insert(new.to_string(), a);
        }
        for m in [&mut self.active_group, &mut self.active_shape] {
            if let Some(v) = m.remove(old) {
                m.insert(new.to_string(), v);
            }
        }
        for s in [&mut self.shape_lock, &mut self.shape_edit_mode] {
            if s.remove(old) {
                s.insert(new.to_string());
            }
        }
        if let Some(v) = self.lattice_sel.remove(old) {
            self.lattice_sel.insert(new.to_string(), v);
        }
        self.group_lock = std::mem::take(&mut self.group_lock).into_iter().map(|(o, g)| (if o == old { new.to_string() } else { o }, g)).collect();
        if let Some((o, _, _)) = &mut self.shape_stash {
            if o == old {
                *o = new.to_string();
            }
        }
    }
    pub fn rename_bone(&mut self, object: &str, old: &str, new: &str) {
        if let Some(a) = self.arms.get_mut(object) {
            if let Some(s) = a.sel.remove(old) {
                a.sel.insert(new.to_string(), s);
            }
            if a.active.as_deref() == Some(old) {
                a.active = Some(new.to_string());
            }
            if a.hidden.remove(old) {
                a.hidden.insert(new.to_string());
            }
        }
    }
}

// ---------------------------------------------------------------- lookups

pub fn is_armature(doc: &Doc, oi: usize) -> bool {
    doc.scene.objects.get(oi).is_some_and(|o| matches!(o.data, ObjectData::Armature(_)))
}
pub fn is_lattice(doc: &Doc, oi: usize) -> bool {
    doc.scene.objects.get(oi).is_some_and(|o| matches!(o.data, ObjectData::Lattice(_)))
}
pub fn is_mesh(doc: &Doc, oi: usize) -> bool {
    doc.scene.objects.get(oi).is_some_and(|o| matches!(o.data, ObjectData::Mesh(_)))
}

/// The armature object in Pose Mode.
pub fn pose_object(doc: &Doc) -> Option<usize> {
    let a = doc.active()?;
    (doc.mode == Mode::Pose && is_armature(doc, a)).then_some(a)
}

/// The armature in Edit Mode: (object, armature data).
pub fn edit_armature(doc: &Doc) -> Option<(usize, usize)> {
    let a = doc.active()?;
    if doc.mode != Mode::Edit {
        return None;
    }
    match doc.scene.objects[a].data {
        ObjectData::Armature(ai) if ai < doc.scene.armatures.len() => Some((a, ai)),
        _ => None,
    }
}

/// The lattice in Edit Mode: (object, lattice data).
pub fn edit_lattice(doc: &Doc) -> Option<(usize, usize)> {
    let a = doc.active()?;
    if doc.mode != Mode::Edit {
        return None;
    }
    match doc.scene.objects[a].data {
        ObjectData::Lattice(li) if li < doc.scene.lattices.len() => Some((a, li)),
        _ => None,
    }
}

/// The armature object whose bones are being edited or posed.
pub fn bone_object(doc: &Doc) -> Option<usize> {
    pose_object(doc).or_else(|| edit_armature(doc).map(|e| e.0))
}

pub fn armature_data(doc: &Doc, oi: usize) -> Option<usize> {
    match doc.scene.objects.get(oi)?.data {
        ObjectData::Armature(ai) if ai < doc.scene.armatures.len() => Some(ai),
        _ => None,
    }
}

pub fn all_bone_names(doc: &Doc, oi: usize) -> Vec<String> {
    doc.scene.armature_of(oi).map(|a| a.bones.iter().map(|b| b.name.clone()).collect()).unwrap_or_default()
}

/// Whether a bone is shown (not hidden, its collections not all hidden).
pub fn bone_visible(ui: &ArmUi, arm: &Armature, i: usize) -> bool {
    let b = &arm.bones[i];
    if ui.hidden.contains(&b.name) {
        return false;
    }
    b.collections.is_empty() || b.collections.iter().any(|c| !ui.hidden_collections.contains(c))
}

/// Selected bones (bodies) of armature object `oi`, in bone order.
pub fn selected_pose_bones(doc: &Doc, oi: usize) -> Vec<String> {
    let name = &doc.scene.objects[oi].name;
    let ui = doc.rig.arms.get(name);
    let Some(arm) = doc.scene.armature_of(oi) else { return Vec::new() };
    arm.bones.iter().filter(|b| ui.is_some_and(|u| u.get(&b.name).body)).map(|b| b.name.clone()).collect()
}

pub fn active_bone(doc: &Doc, oi: usize) -> Option<String> {
    let name = &doc.scene.objects.get(oi)?.name;
    let a = doc.rig.arms.get(name)?.active.clone()?;
    doc.scene.armature_of(oi)?.find(&a).map(|_| a)
}

/// The pose bone `name` of `oi`, created at rest when missing.
pub fn pose_bone_mut<'a>(doc: &'a mut Doc, oi: usize, name: &str) -> &'a mut PoseBone {
    let pose = &mut doc.scene.objects[oi].pose;
    let i = match pose.iter().position(|p| p.name == name) {
        Some(i) => i,
        None => {
            pose.push(PoseBone::new(name));
            pose.len() - 1
        }
    };
    &mut pose[i]
}

pub fn pose_bone(doc: &Doc, oi: usize, name: &str) -> PoseBone {
    doc.scene.objects[oi].pose.iter().find(|p| p.name == name).cloned().unwrap_or_else(|| PoseBone::new(name))
}

/// The channel values that reproduce bone `name`'s pose matrix in `poses`
/// (visual keying, Apply Visual Transform).
pub fn visual_pose_bone(doc: &Doc, oi: usize, name: &str, poses: &[Mat4]) -> Option<PoseBone> {
    let arm = doc.scene.armature_of(oi)?;
    let i = arm.find(name)?;
    let parent = arm.parent(i).and_then(|p| poses.get(p));
    let bpt = arm.parent_transform(i, parent);
    let m = bpt.inverse().apply(poses.get(i)?);
    let mut pb = pose_bone(doc, oi, name);
    pb.set_from_matrix(&m);
    if arm.bones[i].use_connect {
        pb.location = Vec3::ZERO;
    }
    Some(pb)
}

/// The armature deforming mesh object `oi`: its Armature modifier's object,
/// else an armature parent.
pub fn deform_armature(doc: &Doc, oi: usize) -> Option<usize> {
    let o = doc.scene.objects.get(oi)?;
    for m in &o.modifiers {
        if let sex_3d::modifiers::Modifier::Armature { object, .. } = m {
            if let Some(a) = doc.scene.find_object(object).filter(|&a| is_armature(doc, a)) {
                return Some(a);
            }
        }
    }
    o.parent.filter(|&p| is_armature(doc, p))
}

/// Flip a side name: "Arm.L" <-> "Arm.R", "hand_left" <-> "hand_right"
/// (Blender's `BLI_string_flip_side_name`).
pub fn flip_name(name: &str) -> String {
    let pairs: [(&str, &str); 4] = [("Left", "Right"), ("left", "right"), ("LEFT", "RIGHT"), ("Right", "Left")];
    for sep in ['.', '_', '-', ' '] {
        for (a, b) in [("L", "R"), ("R", "L"), ("l", "r"), ("r", "l")] {
            let suffix = format!("{sep}{a}");
            if let Some(stem) = name.strip_suffix(&suffix) {
                return format!("{stem}{sep}{b}");
            }
            let prefix = format!("{a}{sep}");
            if let Some(rest) = name.strip_prefix(&prefix) {
                return format!("{b}{sep}{rest}");
            }
        }
    }
    // Strip a trailing ".001" while looking for whole words.
    for (a, b) in pairs.iter().chain([("right", "left"), ("RIGHT", "LEFT")].iter()) {
        if let Some(p) = name.find(a) {
            return format!("{}{}{}", &name[..p], b, &name[p + a.len()..]);
        }
    }
    name.to_string()
}

// ---------------------------------------------------------------- modes

/// Modes object `oi` can enter.
pub fn modes_for(doc: &Doc, oi: Option<usize>) -> Vec<Mode> {
    let Some(oi) = oi.filter(|&o| o < doc.scene.objects.len()) else { return vec![Mode::Object] };
    match doc.scene.objects[oi].data {
        ObjectData::Mesh(_) => vec![Mode::Object, Mode::Edit, Mode::Sculpt, Mode::VertexPaint, Mode::WeightPaint, Mode::TexturePaint],
        ObjectData::Armature(_) => vec![Mode::Object, Mode::Edit, Mode::Pose],
        ObjectData::Lattice(_) => vec![Mode::Object, Mode::Edit],
        ObjectData::GreasePencil(_) => crate::gpencil::MODES.to_vec(),
        _ => vec![Mode::Object],
    }
}

/// `object.mode_set`: OBJECT, EDIT, POSE, WEIGHT_PAINT... and the
/// TOGGLE_EDIT / TOGGLE_POSE toggles.
pub fn mode_set(doc: &mut Doc, arg: &str) {
    let want = match arg {
        "EDIT" => Mode::Edit,
        "SCULPT" => Mode::Sculpt,
        "VERTEX_PAINT" => Mode::VertexPaint,
        "WEIGHT_PAINT" => Mode::WeightPaint,
        "TEXTURE_PAINT" => Mode::TexturePaint,
        "POSE" => Mode::Pose,
        "PAINT_GREASE_PENCIL" | "PAINT_GPENCIL" | "DRAW" => Mode::Draw,
        "SCULPT_GREASE_PENCIL" | "SCULPT_GPENCIL" => Mode::Sculpt,
        "WEIGHT_GREASE_PENCIL" | "WEIGHT_GPENCIL" => Mode::WeightPaint,
        "VERTEX_GREASE_PENCIL" | "VERTEX_GPENCIL" => Mode::VertexPaint,
        "EDIT_GPENCIL" => Mode::Edit,
        "TOGGLE_EDIT" => {
            if doc.mode == Mode::Edit {
                // Tab from Edit Mode returns to Pose Mode on armatures that
                // came from it; Object Mode otherwise.
                Mode::Object
            } else {
                Mode::Edit
            }
        }
        "TOGGLE_POSE" => {
            if doc.mode == Mode::Pose {
                Mode::Object
            } else {
                Mode::Pose
            }
        }
        "TOGGLE_WEIGHT_PAINT" => {
            if doc.mode == Mode::WeightPaint {
                Mode::Object
            } else {
                Mode::WeightPaint
            }
        }
        _ => Mode::Object,
    };
    if want != Mode::Object && !modes_for(doc, doc.active()).contains(&want) {
        doc.report = match doc.active().map(|a| &doc.scene.objects[a].data) {
            Some(ObjectData::Mesh(_)) | None => format!("{} is not available for this object", want.label()),
            Some(ObjectData::Armature(_)) => format!("Armatures have no {}", want.label()),
            Some(_) => "Active object has no editable data".into(),
        };
        if doc.active().is_none() {
            doc.report = "No active object".into();
        }
        return;
    }
    if want == doc.mode {
        return;
    }
    leave_mode(doc);
    if want == Mode::Edit {
        if let Some(a) = doc.active() {
            match doc.scene.objects[a].data {
                ObjectData::Mesh(_) => {
                    // A fresh mesh in Blender enters Edit Mode all selected.
                    doc.edit = Default::default();
                    doc.edit_hidden.clear();
                    doc.mode = Mode::Edit;
                    shapes::enter_edit(doc, a);
                    crate::edit::select_all(doc, "SELECT");
                }
                ObjectData::Armature(_) => {}
                _ => {}
            }
        }
    }
    doc.mode = want;
    doc.touch();
}

/// Work to do when leaving the current mode.
fn leave_mode(doc: &mut Doc) {
    if doc.mode == Mode::Edit {
        if let Some(a) = doc.active() {
            match doc.scene.objects[a].data {
                ObjectData::Mesh(_) => shapes::leave_edit(doc, a),
                ObjectData::Armature(ai) => armedit::validate(doc, a, ai),
                _ => {}
            }
        }
    }
}

/// Keep rigging state consistent after undo and redo.
pub fn sanitize(doc: &mut Doc) {
    match doc.mode {
        Mode::Pose if pose_object(doc).is_none() => doc.mode = Mode::Object,
        Mode::WeightPaint if !doc.active().is_some_and(|a| is_mesh(doc, a)) && crate::gpencil::active_gp(doc).is_none() => doc.mode = Mode::Object,
        Mode::Draw if crate::gpencil::active_gp(doc).is_none() => doc.mode = Mode::Object,
        _ => {}
    }
}

// ---------------------------------------------------------------- operators

/// Rigging operators. Returns false for names it does not know.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let id = op.id.as_str();
    if id.starts_with("armature.") || id.starts_with("sketch.") {
        return armedit::run(doc, op);
    }
    if id.starts_with("pose.") {
        return pose::run(doc, op);
    }
    if id.starts_with("lattice.") {
        return lattice_op(doc, op);
    }
    if id == "bone.set" || id == "constraint.set" {
        return props::run(doc, op);
    }
    if id.starts_with("constraint.") || id == "object.constraint_add" || id == "pose.constraint_add" {
        return cons::run(doc, op);
    }
    if id.starts_with("paint.") || id.starts_with("object.vertex_group") || id.starts_with("wp.") {
        return paint::run(doc, op);
    }
    if id.starts_with("object.shape_key") || id.starts_with("shape.") {
        return shapes::run(doc, op);
    }
    match id {
        "object.parent_set" => parent_set(doc, &op.arg),
        "object.parent_clear" => {
            doc.push_undo("Clear Parent");
            for i in doc.scene.selected.clone() {
                if op.arg == "CLEAR_KEEP_TRANSFORM" {
                    let w = doc.scene.world_matrix(i);
                    doc.scene.objects[i].transform = Transform::from_matrix(&w);
                }
                if op.arg != "CLEAR_INVERSE" {
                    let o = &mut doc.scene.objects[i];
                    o.parent = None;
                    o.parent_type = ParentType::Object;
                    o.parent_bone.clear();
                }
            }
            doc.touch();
            true
        }
        "rig.set" => {
            set_prop(doc, &op.arg);
            true
        }
        _ => false,
    }
}

/// Ctrl+P: "OBJECT", "ARMATURE" (Armature Deform), "ARMATURE_NAME" (With
/// Empty Groups), "ARMATURE_ENVELOPE", "ARMATURE_AUTO", "BONE",
/// "BONE_RELATIVE"; "KEEP_TRANSFORM" falls through to plain parenting.
fn parent_set(doc: &mut Doc, arg: &str) -> bool {
    use sex_3d::scene::ArmatureWeights as W;
    let weights = match arg {
        "ARMATURE" => W::None,
        "ARMATURE_NAME" => W::Empty,
        "ARMATURE_ENVELOPE" => W::Envelope,
        "ARMATURE_AUTO" => W::Automatic,
        "BONE" | "BONE_RELATIVE" => {
            let Some(a) = doc.active().filter(|&a| is_armature(doc, a)) else {
                doc.report = "Active object is not an armature".into();
                return true;
            };
            let Some(bone) = active_bone(doc, a) else {
                doc.report = "No active bone".into();
                return true;
            };
            doc.push_undo("Make Parent");
            let mut n = 0;
            for i in doc.scene.selected.clone() {
                if i != a && doc.scene.parent_to_bone(i, a, &bone, true) {
                    if arg == "BONE_RELATIVE" {
                        if let Some(ai) = armature_data(doc, a) {
                            if let Some(bi) = doc.scene.armatures[ai].find(&bone) {
                                doc.scene.armatures[ai].bones[bi].use_relative_parent = true;
                            }
                        }
                    }
                    n += 1;
                }
            }
            doc.report = format!("Parented {n} object(s) to bone '{bone}'");
            doc.touch();
            return true;
        }
        _ => {
            // Plain parenting; keep the type right when re-parenting.
            if let Some(a) = doc.active() {
                for i in doc.scene.selected.clone() {
                    if i != a {
                        doc.scene.objects[i].parent_type = ParentType::Object;
                        doc.scene.objects[i].parent_bone.clear();
                    }
                }
            }
            return false;
        }
    };
    let Some(a) = doc.active().filter(|&a| is_armature(doc, a)) else {
        doc.report = "Active object is not an armature".into();
        return true;
    };
    doc.push_undo("Make Parent");
    let mut n = 0;
    for i in doc.scene.selected.clone() {
        if i == a {
            continue;
        }
        if is_mesh(doc, i) {
            if doc.scene.parent_with_armature(i, a, weights) {
                n += 1;
            }
        } else {
            let pw = doc.scene.world_matrix(a);
            let local = pw.inverse() * doc.scene.world_matrix(i);
            let o = &mut doc.scene.objects[i];
            o.parent = Some(a);
            o.parent_type = ParentType::Object;
            o.transform = Transform::from_matrix(&local);
            n += 1;
        }
    }
    doc.report = format!("Parented {n} object(s) to the armature");
    doc.touch();
    true
}

/// `rig.set key=value` from the Properties editor and menus.
fn set_prop(doc: &mut Doc, arg: &str) {
    let Some((key, v)) = arg.split_once('=') else { return };
    let flag = |cur: bool| match v {
        "TOGGLE" => !cur,
        x => x == "1" || x == "true" || x == "True",
    };
    let arm_obj = doc.active().filter(|&a| is_armature(doc, a)).map(|a| doc.scene.objects[a].name.clone());
    match key {
        "auto_ik" => doc.rig.auto_ik = flag(doc.rig.auto_ik),
        "wp.brush" => {
            if let Some(b) = WpBrush::from_label(v) {
                doc.rig.wp.brush = b;
            }
        }
        "wp.blend" => {
            if let Some(b) = WpBlend::from_label(v) {
                doc.rig.wp.blend = b;
            }
        }
        "wp.weight" => doc.rig.wp.weight = v.parse::<f32>().unwrap_or(doc.rig.wp.weight).clamp(0.0, 1.0),
        "wp.strength" => doc.rig.wp.strength = v.parse::<f32>().unwrap_or(doc.rig.wp.strength).clamp(0.0, 1.0),
        "wp.radius" => doc.rig.wp.radius = v.parse::<f32>().unwrap_or(doc.rig.wp.radius).clamp(1.0, 500.0),
        "wp.auto_normalize" => doc.rig.wp.auto_normalize = flag(doc.rig.wp.auto_normalize),
        "wp.x_mirror" => doc.rig.wp.x_mirror = flag(doc.rig.wp.x_mirror),
        "vg_weight" => doc.rig.wp.vg_weight = v.parse::<f32>().unwrap_or(doc.rig.wp.vg_weight).clamp(0.0, 1.0),
        "display_type" | "show_names" | "show_axes" | "show_in_front" => {
            let Some(n) = arm_obj else { return };
            let ui = doc.rig.arm_mut(&n);
            match key {
                "display_type" => {
                    if let Some(dt) = BoneDisplay::from_id(v) {
                        ui.display = dt;
                    }
                }
                "show_names" => ui.show_names = flag(ui.show_names),
                "show_axes" => ui.show_axes = flag(ui.show_axes),
                _ => ui.show_in_front = flag(ui.show_in_front),
            }
            doc.touch();
        }
        "pose_position" => {
            let Some(a) = doc.active() else { return };
            let Some(ai) = armature_data(doc, a) else { return };
            doc.push_undo("Pose Position");
            doc.scene.armatures[ai].rest_position = v == "REST";
            doc.touch();
        }
        _ => {}
    }
}

/// Lattice Edit Mode operators.
fn lattice_op(doc: &mut Doc, op: &Op) -> bool {
    let Some((oi, li)) = edit_lattice(doc) else {
        if op.id == "lattice.resize" || op.id == "lattice.interpolation" {
            if let Some(li) = doc.active().and_then(|a| match doc.scene.objects[a].data {
                ObjectData::Lattice(l) => Some(l),
                _ => None,
            }) {
                lattice_props(doc, li, op);
            }
        }
        return op.id.starts_with("lattice.");
    };
    let name = doc.scene.objects[oi].name.clone();
    let n = doc.scene.lattices[li].points.len();
    match op.id.as_str() {
        "lattice.select_all" => {
            let sel = doc.rig.lattice_sel.entry(name).or_default();
            match op.arg.as_str() {
                "DESELECT" => sel.clear(),
                "INVERT" => *sel = (0..n).filter(|i| !sel.contains(i)).collect(),
                "TOGGLE" if !sel.is_empty() => sel.clear(),
                _ => *sel = (0..n).collect(),
            }
        }
        "lattice.select_mirror" => {
            let l = &doc.scene.lattices[li];
            let sel = doc.rig.lattice_sel.entry(name).or_default();
            let mut out = BTreeSet::new();
            for &i in sel.iter() {
                let (u, v, w) = (i as u32 % l.points_u, (i as u32 / l.points_u) % l.points_v, i as u32 / (l.points_u * l.points_v));
                out.insert(l.index(l.points_u - 1 - u, v, w));
            }
            *sel = out;
        }
        "lattice.make_regular" => {
            doc.push_undo("Make Regular");
            let l = &mut doc.scene.lattices[li];
            l.points = l.rest_points();
            doc.touch();
        }
        "lattice.flip" => {
            doc.push_undo("Flip");
            let axis = match op.arg.as_str() {
                "V" => 1,
                "W" => 2,
                _ => 0,
            };
            let l = &mut doc.scene.lattices[li];
            let old = l.points.clone();
            for w in 0..l.points_w {
                for v in 0..l.points_v {
                    for u in 0..l.points_u {
                        let (fu, fv, fw) = match axis {
                            0 => (l.points_u - 1 - u, v, w),
                            1 => (u, l.points_v - 1 - v, w),
                            _ => (u, v, l.points_w - 1 - w),
                        };
                        let mut p = old[l.index(fu, fv, fw)];
                        match axis {
                            0 => p.x = -p.x,
                            1 => p.y = -p.y,
                            _ => p.z = -p.z,
                        }
                        let i = l.index(u, v, w);
                        l.points[i] = p;
                    }
                }
            }
            doc.touch();
        }
        "lattice.resize" | "lattice.interpolation" => lattice_props(doc, li, op),
        _ => return false,
    }
    true
}

/// "lattice.resize" "U V W" and "lattice.interpolation" "AXIS=KIND".
fn lattice_props(doc: &mut Doc, li: usize, op: &Op) {
    use sex_3d::mesh::KeyInterpolation as K;
    if op.id == "lattice.resize" {
        let v: Vec<u32> = op.arg.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if v.len() == 3 {
            doc.push_undo("Lattice Resolution");
            let (u, vv, w) = (v[0].clamp(1, 64), v[1].clamp(1, 64), v[2].clamp(1, 64));
            doc.scene.lattices[li].resize(u, vv, w);
            doc.rig.lattice_sel.clear();
            doc.touch();
        }
        return;
    }
    let Some((axis, kind)) = op.arg.split_once('=') else { return };
    let k = match kind {
        "KEY_CARDINAL" | "Cardinal" => K::Cardinal,
        "KEY_CATMULL_ROM" | "Catmull-Rom" => K::CatmullRom,
        "KEY_BSPLINE" | "BSpline" => K::BSpline,
        _ => K::Linear,
    };
    doc.push_undo("Lattice Interpolation");
    let l = &mut doc.scene.lattices[li];
    match axis {
        "U" => l.interpolation_u = k,
        "V" => l.interpolation_v = k,
        _ => l.interpolation_w = k,
    }
    doc.touch();
}

pub fn interp_label(k: sex_3d::mesh::KeyInterpolation) -> &'static str {
    use sex_3d::mesh::KeyInterpolation as K;
    match k {
        K::Linear => "Linear",
        K::Cardinal => "Cardinal",
        K::CatmullRom => "Catmull-Rom",
        K::BSpline => "BSpline",
    }
}

/// The `--screenshot --rig-demo` scene: a skinned column on a three-bone
/// chain, keyed to sway, shown in Pose Mode.
pub fn demo(doc: &mut Doc) {
    if let Some(c) = doc.scene.find_object("Cube") {
        doc.select_only(Some(c));
        doc.run(&Op::new("object.delete"));
    }
    let (seg, rings, r, h) = (12u32, 13u32, 0.35f32, 2.4f32);
    let mut pos = Vec::new();
    for j in 0..rings {
        let t = j as f32 / (rings - 1) as f32;
        let rad = r * (1.0 - 0.35 * t);
        for i in 0..seg {
            let a = i as f32 / seg as f32 * std::f32::consts::TAU;
            pos.push(Vec3::new(rad * a.cos(), rad * a.sin(), t * h));
        }
    }
    let mut faces: Vec<Vec<u32>> = Vec::new();
    for j in 0..rings - 1 {
        for i in 0..seg {
            let n = (i + 1) % seg;
            faces.push(vec![j * seg + i, j * seg + n, (j + 1) * seg + n, (j + 1) * seg + i]);
        }
    }
    faces.push((0..seg).rev().collect());
    faces.push(((rings - 1) * seg..rings * seg).collect());
    let mut mesh = sex_3d::mesh::Mesh::from_faces(pos, faces);
    for f in &mut mesh.faces {
        f.smooth = true;
    }
    let body = doc.scene.add_mesh_object("Body", mesh);
    let mut arm = Armature::default();
    let names = ["Root", "Spine", "Neck"];
    for (k, n) in names.iter().enumerate() {
        let mut b = sex_3d::armature::Bone::new(n, Vec3::new(0.0, 0.0, k as f32 * 0.8), Vec3::new(0.0, 0.0, (k + 1) as f32 * 0.8));
        if k > 0 {
            b.parent = Some(k - 1);
            b.use_connect = true;
        }
        arm.add_bone(b);
    }
    let a = doc.scene.add_armature_object("Armature", arm);
    doc.scene.selected = vec![body, a];
    doc.scene.active_object = Some(a);
    doc.run(&Op::with("object.parent_set", "ARMATURE_AUTO"));
    doc.select_only(Some(a));
    mode_set(doc, "POSE");
    let obj = doc.scene.objects[a].name.clone();
    let ui = doc.rig.arm_mut(&obj);
    for n in names {
        ui.set(n, BoneSel::ALL);
    }
    ui.active = Some("Spine".into());
    ui.show_names = true;
    for (bone, amount) in [("Spine", 0.5f32), ("Neck", 0.7)] {
        let path = format!("pose.bones[\"{bone}\"].rotation_quaternion");
        for (frame, sign) in [(1.0, -1.0f32), (20.0, 1.0), (40.0, -1.0)] {
            let half = sign * amount * 0.5;
            crate::anim::insert_key_value(doc, a, &path, 0, frame, half.cos());
            crate::anim::insert_key_value(doc, a, &path, 1, frame, half.sin());
        }
    }
    doc.scene.frame_end = 40;
    doc.frame = 12;
    crate::anim::evaluate(doc);
    doc.touch();
}
