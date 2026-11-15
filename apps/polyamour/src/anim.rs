// SPDX-License-Identifier: GPL-3.0-only
//! Animation, Blender's way: actions of F-curves on data paths
//! ([`path`]: object transforms, pose bones, shape keys, light, camera,
//! material, modifier and constraint numbers), NLA tracks of action strips
//! ([`nla`]), drivers with scripted expressions ([`driver`], [`expr`]),
//! keying sets, auto-keying and the keyframe operators the animation editors
//! share. Evaluating at the current frame writes the animated values into
//! the scene; that happens whenever the frame or the animation changes
//! ([`tick`]), drivers on every frame.
//!
//! The module also keeps the report history the Info editor lists, the
//! in-program clipboard the text editors share and a small mailbox that routes
//! menu operators back to the editor whose header opened the menu.
pub mod curve;
pub mod driver;
pub mod expr;
pub mod nla;
pub mod path;
pub mod props;

pub use curve::{Action, CycleMode, Easing, Extrapolation, FCurve, FModifier, HandleType, Interp, KeyType, Keyframe, NoiseBlend};
pub use driver::{Driver, DriverTarget, DriverType, DriverVar, TransformChannel, TransformSpace, VarType};
pub use nla::{Blend, NlaStrip, NlaTrack, StripExtrap};

use crate::doc::{Doc, Mode};
use crate::ui::Op;
use sex_3d::scene::{ObjectData, Scene};
use sex_3d::Transform;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

/// One object's animation data (Blender's `AnimData`): the active action,
/// its blending, NLA tracks (bottom first), drivers and tweak mode.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectAnim {
    /// Objects are matched by name (names are unique in a scene).
    pub object: String,
    /// Index into [`Anim::actions`].
    pub action: Option<usize>,
    pub blend: Blend,
    pub influence: f32,
    pub tracks: Vec<NlaTrack>,
    pub drivers: Vec<Driver>,
    /// NLA tweak mode: (track, strip) whose action is being edited.
    pub tweak: Option<(usize, usize)>,
    /// Channel-list disclosure: the object row, and groups closed by name.
    pub expanded: bool,
    pub collapsed: BTreeSet<String>,
}

impl ObjectAnim {
    pub fn new(object: &str) -> ObjectAnim {
        ObjectAnim {
            object: object.into(),
            action: None,
            blend: Blend::Replace,
            influence: 1.0,
            tracks: Vec::new(),
            drivers: Vec::new(),
            tweak: None,
            expanded: true,
            collapsed: BTreeSet::new(),
        }
    }
    /// The action keys go into: the tweaked strip's, else the active one.
    pub fn edit_action(&self) -> Option<usize> {
        match self.tweak {
            Some((t, s)) => self.tracks.get(t).and_then(|t| t.strips.get(s)).map(|s| s.action),
            None => self.action,
        }
    }
    /// Action time of scene frame `f` for the edited action (NLA mapping in
    /// tweak mode).
    pub fn action_time(&self, f: f32) -> f32 {
        match self.tweak.and_then(|(t, s)| self.tracks.get(t)?.strips.get(s)) {
            Some(s) => s.action_time(f),
            None => f,
        }
    }
    /// Scene frame of action time `t` for the edited action.
    pub fn scene_time(&self, t: f32) -> f32 {
        match self.tweak.and_then(|(tr, s)| self.tracks.get(tr)?.strips.get(s)) {
            Some(s) => s.scene_time(t),
            None => t,
        }
    }
    fn is_empty(&self, actions: &[Action]) -> bool {
        self.tracks.is_empty() && self.drivers.is_empty() && self.action.is_none_or(|a| actions.get(a).is_none_or(|a| a.curves.is_empty()))
    }
}

/// What a property field shows: keyed on this frame (yellow), animated
/// (green), driven (purple) or nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldState {
    None,
    Animated,
    Keyed,
    Driven,
}

/// All animation in the document.
#[derive(Clone, Debug)]
pub struct Anim {
    pub actions: Vec<Action>,
    pub objects: Vec<ObjectAnim>,
    /// The timeline's record button (auto-keying).
    pub auto_key: bool,
    /// Playing backwards (Shift+Ctrl+Space / the reverse play button).
    pub reverse: bool,
    /// Interpolation, handle and keyframe type of new keys.
    pub interp: Interp,
    pub handle: HandleType,
    pub key_type: KeyType,
    /// The active keying set (K), by id; None uses the I menu.
    pub keying_set: Option<String>,
    /// Keys or curves changed: evaluate again even if the frame did not.
    pub dirty: bool,
    last_frame: Option<i32>,
    /// Transforms of selected objects after the last evaluation, for auto-keying.
    seen: Vec<(String, Transform)>,
}

impl Default for Anim {
    fn default() -> Self {
        Anim {
            actions: Vec::new(),
            objects: Vec::new(),
            auto_key: false,
            reverse: false,
            interp: Interp::Bezier,
            handle: HandleType::AutoClamped,
            key_type: KeyType::Keyframe,
            keying_set: None,
            dirty: false,
            last_frame: None,
            seen: Vec::new(),
        }
    }
}

/// A key by position: action, curve, key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeyRef {
    pub a: usize,
    pub c: usize,
    pub k: usize,
}

impl Anim {
    pub fn get(&self, object: &str) -> Option<&ObjectAnim> {
        self.objects.iter().find(|o| o.object == object)
    }
    pub fn get_mut(&mut self, object: &str) -> Option<&mut ObjectAnim> {
        self.objects.iter_mut().find(|o| o.object == object)
    }
    /// Index of the object's animation data, created when missing.
    pub fn entry(&mut self, object: &str) -> usize {
        match self.objects.iter().position(|o| o.object == object) {
            Some(i) => i,
            None => {
                self.objects.push(ObjectAnim::new(object));
                self.objects.len() - 1
            }
        }
    }
    /// The action keys of `object` go into, created ("CubeAction") if needed.
    pub fn ensure_action(&mut self, object: &str) -> usize {
        let o = self.entry(object);
        if let Some(a) = self.objects[o].edit_action().filter(|&a| a < self.actions.len()) {
            return a;
        }
        let name = sex_3d::scene::unique_name(&format!("{object}Action"), |n| self.actions.iter().any(|a| a.name == n));
        self.actions.push(Action::new(&name));
        let a = self.actions.len() - 1;
        self.objects[o].action = Some(a);
        a
    }
    /// The action being edited for `object`, if any.
    pub fn edit_action(&self, object: &str) -> Option<usize> {
        self.get(object)?.edit_action().filter(|&a| a < self.actions.len())
    }
    /// Curves of the edited action of `object`.
    pub fn curves(&self, object: &str) -> &[FCurve] {
        match self.edit_action(object) {
            Some(a) => &self.actions[a].curves,
            None => &[],
        }
    }
    /// Keyed frames of `object`'s edited action, in scene time.
    pub fn key_frames(&self, object: &str) -> Vec<f32> {
        let Some(oa) = self.get(object) else { return Vec::new() };
        let Some(a) = self.edit_action(object) else { return Vec::new() };
        self.actions[a].key_frames().into_iter().map(|f| oa.scene_time(f)).collect()
    }
    /// Follow an object rename: its curves keep driving it under the new name.
    pub fn rename_object(&mut self, old: &str, new: &str) {
        for o in self.objects.iter_mut().filter(|o| o.object == old) {
            o.object = new.to_string();
        }
        for o in &mut self.objects {
            for d in &mut o.drivers {
                for v in &mut d.vars {
                    for t in &mut v.targets {
                        if t.object == old {
                            t.object = new.to_string();
                        }
                    }
                }
            }
        }
        for (n, _) in self.seen.iter_mut().filter(|(n, _)| n == old) {
            *n = new.to_string();
        }
        self.dirty = true;
    }
    /// Follow a bone rename in `object`'s paths.
    pub fn rename_bone(&mut self, object: &str, old: &str, new: &str) {
        let from = format!("pose.bones[{}]", path::quote(old));
        let to = format!("pose.bones[{}]", path::quote(new));
        let Some(oa) = self.get(object) else { return };
        let mut acts: Vec<usize> = oa.action.into_iter().collect();
        acts.extend(oa.tracks.iter().flat_map(|t| t.strips.iter().map(|s| s.action)));
        for a in acts {
            if let Some(act) = self.actions.get_mut(a) {
                for c in &mut act.curves {
                    if c.path.starts_with(&from) {
                        c.path = c.path.replacen(&from, &to, 1);
                        if c.group == old {
                            c.group = new.to_string();
                        }
                    }
                }
            }
        }
        if let Some(oa) = self.get_mut(object) {
            for d in &mut oa.drivers {
                if d.path.starts_with(&from) {
                    d.path = d.path.replacen(&from, &to, 1);
                }
            }
        }
        self.dirty = true;
    }
    /// Every key of every action, for selection passes.
    pub fn keys_mut(&mut self) -> impl Iterator<Item = &mut Keyframe> {
        self.actions.iter_mut().flat_map(|a| a.curves.iter_mut().flat_map(|c| c.keys.iter_mut()))
    }
    pub fn any_selected_key(&self) -> bool {
        self.actions.iter().any(|a| a.curves.iter().any(|c| c.keys.iter().any(|k| k.any_selected())))
    }
    /// Selected keys, in key order.
    pub fn selected_keys(&self) -> Vec<KeyRef> {
        let mut out = Vec::new();
        for (a, act) in self.actions.iter().enumerate() {
            for (c, fc) in act.curves.iter().enumerate() {
                for (k, key) in fc.keys.iter().enumerate() {
                    if key.select {
                        out.push(KeyRef { a, c, k });
                    }
                }
            }
        }
        out
    }
    pub fn key(&self, r: KeyRef) -> Option<&Keyframe> {
        self.actions.get(r.a)?.curves.get(r.c)?.keys.get(r.k)
    }
    pub fn key_mut(&mut self, r: KeyRef) -> Option<&mut Keyframe> {
        self.actions.get_mut(r.a)?.curves.get_mut(r.c)?.keys.get_mut(r.k)
    }
    /// Users of action `a` (active actions and strips).
    pub fn action_users(&self, a: usize) -> usize {
        self.objects.iter().map(|o| (o.action == Some(a)) as usize + o.tracks.iter().flat_map(|t| &t.strips).filter(|s| s.action == a).count()).sum()
    }
    /// Drop animation data with nothing in it, and empty actions nothing uses.
    pub fn gc(&mut self) {
        let acts = &self.actions;
        self.objects.retain(|o| !o.is_empty(acts));
        for o in &mut self.objects {
            if o.action.is_some_and(|a| a >= self.actions.len()) {
                o.action = None;
            }
        }
        let mut a = 0;
        while a < self.actions.len() {
            let act = &self.actions[a];
            if act.curves.is_empty() && !act.fake_user && self.action_users(a) == 0 {
                self.remove_action(a);
            } else {
                a += 1;
            }
        }
    }
    /// Remove action `a`, fixing indices (users of it lose it).
    pub fn remove_action(&mut self, a: usize) {
        self.actions.remove(a);
        let fix = |x: usize| if x > a { x - 1 } else { x };
        for o in &mut self.objects {
            o.action = match o.action {
                Some(x) if x == a => None,
                Some(x) => Some(fix(x)),
                None => None,
            };
            for t in &mut o.tracks {
                t.strips.retain(|s| s.action != a);
                for s in &mut t.strips {
                    s.action = fix(s.action);
                }
            }
            if o.tweak.is_some_and(|(t, s)| o.tracks.get(t).and_then(|t| t.strips.get(s)).is_none()) {
                o.tweak = None;
            }
        }
    }
    pub fn driver(&self, object: &str, path: &str, index: usize) -> Option<&Driver> {
        self.get(object)?.drivers.iter().find(|d| d.path == path && d.index == index)
    }
    /// How a property field should look.
    pub fn field_state(&self, object: &str, path: &str, index: usize, frame: i32) -> FieldState {
        let Some(oa) = self.get(object) else { return FieldState::None };
        if oa.drivers.iter().any(|d| d.path == path && d.index == index) {
            return FieldState::Driven;
        }
        let Some(a) = oa.edit_action() else { return FieldState::None };
        let Some(c) = self.actions.get(a).and_then(|act| act.curves.iter().find(|c| c.path == path && c.index == index)) else { return FieldState::None };
        let t = oa.action_time(frame as f32);
        if c.keys.iter().any(|k| (k.frame - t).abs() < 0.01) {
            FieldState::Keyed
        } else {
            FieldState::Animated
        }
    }
}

/// Insert a key on `object` (scene object `oi`) for `path[index]` at scene
/// `frame` with `value`. Returns false for paths that do not resolve.
pub fn insert_key_value(doc: &mut Doc, oi: usize, path: &str, index: usize, frame: f32, value: f32) -> bool {
    let Some(name) = doc.scene.objects.get(oi).map(|o| o.name.clone()) else { return false };
    let (interp, handle, key_type) = (doc.anim.interp, doc.anim.handle, doc.anim.key_type);
    let a = doc.anim.ensure_action(&name);
    let t = doc.anim.get(&name).map_or(frame, |o| o.action_time(frame));
    let c = doc.anim.actions[a].curve_mut(path, index);
    if c.lock {
        return false;
    }
    let k = c.insert(t, value, interp);
    if c.keys[k].h1 != handle || c.keys[k].key_type != key_type {
        c.keys[k].h1 = handle;
        c.keys[k].h2 = handle;
        c.keys[k].key_type = key_type;
        c.recalc_handles();
    }
    doc.anim.dirty = true;
    true
}

/// Insert keys on `path` (every element, or `index`) from the property's
/// current value. Returns how many keys went in.
pub fn insert_path(doc: &mut Doc, oi: usize, path: &str, index: Option<usize>, frame: f32) -> usize {
    let Some(vals) = path::get_array(&doc.scene, oi, path) else { return 0 };
    let mut n = 0;
    for (i, v) in vals.into_iter().enumerate() {
        if index.is_some_and(|x| x != i) {
            continue;
        }
        if doc.anim.get(&doc.scene.objects[oi].name).is_some_and(|o| o.drivers.iter().any(|d| d.path == path && d.index == i)) {
            continue;
        }
        n += insert_key_value(doc, oi, path, i, frame, v) as usize;
    }
    n
}

/// Blender's built-in keying sets: (id, name).
pub const KEYING_SETS: [(&str, &str); 17] = [
    ("Location", "Location"),
    ("Rotation", "Rotation"),
    ("Scaling", "Scale"),
    ("LocRot", "Location & Rotation"),
    ("LocScale", "Location & Scale"),
    ("LocRotScale", "Location, Rotation & Scale"),
    ("RotScale", "Rotation & Scale"),
    ("Available", "Available"),
    ("BUILTIN_KSI_VisualLoc", "Visual Location"),
    ("BUILTIN_KSI_VisualRot", "Visual Rotation"),
    ("BUILTIN_KSI_VisualScaling", "Visual Scale"),
    ("BUILTIN_KSI_VisualLocRot", "Visual Location & Rotation"),
    ("BUILTIN_KSI_VisualLocScale", "Visual Location & Scale"),
    ("BUILTIN_KSI_VisualLocRotScale", "Visual Location, Rotation & Scale"),
    ("BUILTIN_KSI_VisualRotScale", "Visual Rotation & Scale"),
    ("WholeCharacter", "Whole Character"),
    ("WholeCharacterSelected", "Whole Character (Selected Bones Only)"),
];

pub fn keying_set_name(id: &str) -> &'static str {
    KEYING_SETS.iter().find(|k| k.0 == id).map_or("Location, Rotation & Scale", |k| k.1)
}

/// (location, rotation, scale, visual, available, whole character) for a
/// keying set id (old ids like "LOCATION" still work).
fn keying_flags(id: &str) -> (bool, bool, bool, bool, bool, bool) {
    let id = match id {
        "LOCATION" => "Location",
        "ROTATION" => "Rotation",
        "Scale" | "SCALE" => "Scaling",
        "" => "LocRotScale",
        x => x,
    };
    let visual = id.starts_with("BUILTIN_KSI_Visual");
    let core = id.trim_start_matches("BUILTIN_KSI_Visual");
    match core {
        "Available" => (false, false, false, false, true, false),
        "WholeCharacter" | "WholeCharacterSelected" => (true, true, true, false, false, true),
        _ => {
            let loc = core.starts_with("Loc") || core == "Location";
            let rot = core.contains("Rot") || core == "Rotation";
            let scale = core.contains("Scal");
            (loc, rot, scale, visual, false, false)
        }
    }
}

/// Insert keys with keying set `id` ("" = the default LocRotScale) on the
/// selection: selected pose bones in Pose Mode, else selected objects.
/// Returns how many objects or bones got keys.
pub fn insert_keying_set(doc: &mut Doc, frame: f32, id: &str) -> usize {
    let (loc, rot, scale, visual, available, whole) = keying_flags(id);
    let mut count = 0;
    if doc.mode == Mode::Pose {
        let Some(oi) = crate::rig::pose_object(doc) else { return 0 };
        let bones: Vec<String> = if whole && id == "WholeCharacter" { crate::rig::all_bone_names(doc, oi) } else { crate::rig::selected_pose_bones(doc, oi) };
        let poses = visual.then(|| doc.scene.pose_matrices(oi));
        for b in bones {
            let mut keyed = 0;
            let mode = doc.scene.objects[oi].pose.iter().find(|p| p.name == b).map_or(Default::default(), |p| p.rotation_mode);
            if available {
                let name = doc.scene.objects[oi].name.clone();
                let paths: BTreeSet<(String, usize)> = doc.anim.curves(&name).iter().filter(|c| path::bone_of(&c.path).as_deref() == Some(&b)).map(|c| (c.path.clone(), c.index)).collect();
                for (p, i) in paths {
                    keyed += insert_path(doc, oi, &p, Some(i), frame);
                }
            } else if let Some(poses) = &poses {
                // Visual keying: the constrained pose, as channel values.
                let Some(pb) = crate::rig::visual_pose_bone(doc, oi, &b, poses) else { continue };
                let rp = path::bone_rotation_prop(pb.rotation_mode);
                let mut sets: Vec<(String, Vec<f32>)> = Vec::new();
                if loc {
                    sets.push(("location".into(), pb.location.to_array().to_vec()));
                }
                if rot {
                    let v = match pb.rotation_mode {
                        sex_3d::armature::RotationMode::Quaternion => vec![pb.rotation_quaternion.w, pb.rotation_quaternion.x, pb.rotation_quaternion.y, pb.rotation_quaternion.z],
                        sex_3d::armature::RotationMode::Euler(_) => pb.rotation_euler.to_array().to_vec(),
                        sex_3d::armature::RotationMode::AxisAngle => pb.rotation_axis_angle.to_vec(),
                    };
                    sets.push((rp.into(), v));
                }
                if scale {
                    sets.push(("scale".into(), pb.scale.to_array().to_vec()));
                }
                for (prop, vals) in sets {
                    let p = path::bone_path(&b, &prop);
                    for (i, v) in vals.into_iter().enumerate() {
                        keyed += insert_key_value(doc, oi, &p, i, frame, v) as usize;
                    }
                }
            } else {
                if loc {
                    keyed += insert_path(doc, oi, &path::bone_path(&b, "location"), None, frame);
                }
                if rot {
                    keyed += insert_path(doc, oi, &path::bone_path(&b, path::bone_rotation_prop(mode)), None, frame);
                }
                if scale {
                    keyed += insert_path(doc, oi, &path::bone_path(&b, "scale"), None, frame);
                }
            }
            count += (keyed > 0) as usize;
        }
        return count;
    }
    for oi in doc.scene.selected.clone() {
        if oi >= doc.scene.objects.len() {
            continue;
        }
        let name = doc.scene.objects[oi].name.clone();
        let mut keyed = 0;
        if available {
            let paths: Vec<(String, usize)> = doc.anim.curves(&name).iter().map(|c| (c.path.clone(), c.index)).collect();
            for (p, i) in paths {
                keyed += insert_path(doc, oi, &p, Some(i), frame);
            }
        } else {
            let rp = match doc.settings.rotation_mode(&name) {
                crate::settings::RotationMode::Quaternion => "rotation_quaternion",
                crate::settings::RotationMode::AxisAngle => "rotation_axis_angle",
                crate::settings::RotationMode::EulerXyz => "rotation_euler",
            };
            let vis = visual.then(|| {
                let w = doc.scene.world_matrix(oi);
                let local = match doc.scene.objects[oi].parent {
                    Some(p) => doc.scene.world_matrix(p).inverse() * w,
                    None => w,
                };
                Transform::from_matrix(&local)
            });
            let write = |doc: &mut Doc, p: &str| -> usize {
                match vis {
                    Some(t) => {
                        let mut tmp = doc.scene.objects[oi].clone();
                        tmp.transform = t;
                        let saved = core::mem::replace(&mut doc.scene.objects[oi], tmp);
                        let vals = path::get_array(&doc.scene, oi, p).unwrap_or_default();
                        doc.scene.objects[oi] = saved;
                        vals.into_iter().enumerate().map(|(i, v)| insert_key_value(doc, oi, p, i, frame, v) as usize).sum()
                    }
                    None => insert_path(doc, oi, p, None, frame),
                }
            };
            if loc {
                keyed += write(doc, "location");
            }
            if rot {
                keyed += write(doc, rp);
            }
            if scale {
                keyed += write(doc, "scale");
            }
        }
        count += (keyed > 0) as usize;
    }
    count
}

/// Insert keys on the selected objects at `frame` with keying set `only`
/// (None = Location, Rotation & Scale). Returns how many were keyed.
pub fn insert_keys(doc: &mut Doc, frame: i32, only: Option<&str>) -> usize {
    insert_keying_set(doc, frame as f32, only.unwrap_or(""))
}

/// Auto-keying after a transform: key what moved (Location, Rotation and
/// Scale) when the record button is on.
pub fn autokey_objects(doc: &mut Doc, objects: &[usize]) {
    if !doc.anim.auto_key {
        return;
    }
    let keep = core::mem::replace(&mut doc.scene.selected, objects.to_vec());
    let mode = core::mem::replace(&mut doc.mode, Mode::Object);
    insert_keying_set(doc, doc.frame as f32, doc.anim.keying_set.clone().as_deref().unwrap_or(""));
    doc.mode = mode;
    doc.scene.selected = keep;
    doc.anim.seen = snapshot(doc);
}

/// Auto-keying for pose bones after a transform.
pub fn autokey_bones(doc: &mut Doc) {
    if !doc.anim.auto_key || doc.mode != Mode::Pose {
        return;
    }
    let id = doc.anim.keying_set.clone().unwrap_or_default();
    insert_keying_set(doc, doc.frame as f32, &id);
}

/// Delete keys at `frame` on the selection (Alt+I).
pub fn delete_keys_at(doc: &mut Doc, frame: f32) -> usize {
    let mut targets: Vec<(String, Option<String>)> = Vec::new();
    if doc.mode == Mode::Pose {
        if let Some(oi) = crate::rig::pose_object(doc) {
            let name = doc.scene.objects[oi].name.clone();
            for b in crate::rig::selected_pose_bones(doc, oi) {
                targets.push((name.clone(), Some(b)));
            }
        }
    } else {
        targets = doc.scene.selected.iter().filter_map(|&i| doc.scene.objects.get(i)).map(|o| (o.name.clone(), None)).collect();
    }
    let mut n = 0;
    for (name, bone) in targets {
        let Some(oa) = doc.anim.get(&name) else { continue };
        let t = oa.action_time(frame);
        let Some(a) = oa.edit_action() else { continue };
        let Some(act) = doc.anim.actions.get_mut(a) else { continue };
        for c in &mut act.curves {
            if bone.as_ref().is_some_and(|b| path::bone_of(&c.path).as_ref() != Some(b)) {
                continue;
            }
            let before = c.keys.len();
            c.keys.retain(|k| (k.frame - t).abs() >= 0.01);
            n += before - c.keys.len();
            c.recalc_handles();
        }
        act.curves.retain(|c| !c.keys.is_empty());
    }
    doc.anim.gc();
    doc.anim.dirty = true;
    n
}

/// Evaluate everything at the document's frame. Returns true when a value
/// changed.
pub fn evaluate(doc: &mut Doc) -> bool {
    let (changed, results) = pose_scene_full(&doc.anim, &mut doc.scene, doc.frame as f32);
    for (o, d, v, e) in results {
        if let Some(drv) = doc.anim.objects.get_mut(o).and_then(|oa| oa.drivers.get_mut(d)) {
            drv.value = v;
            drv.error = e;
        }
    }
    if changed {
        doc.touch();
    }
    changed
}

/// A copy of `scene` posed at `frame`, for rendering. With `motion`, animated
/// objects also carry their pose one frame later (for motion blur).
pub fn scene_at(anim: &Anim, scene: &Scene, frame: f32, motion: bool) -> Scene {
    let mut s = scene.clone();
    pose_scene(anim, &mut s, frame);
    s.frame_current = frame.round() as i32;
    if motion {
        let mut next = s.clone();
        pose_scene(anim, &mut next, frame + 1.0);
        for (o, n) in s.objects.iter_mut().zip(&next.objects) {
            if n.transform != o.transform {
                o.motion = Some(n.transform);
            }
        }
    }
    s
}

/// The values the NLA stack and active action give `oa` at frame `f`.
pub fn channels(anim: &Anim, oa: &ObjectAnim, f: f32) -> nla::Channels {
    let mut acc = nla::Channels::new();
    let solo = oa.tracks.iter().any(|t| t.solo);
    let top = match oa.tweak {
        Some((t, _)) => (t + 1).min(oa.tracks.len()),
        None => oa.tracks.len(),
    };
    for t in &oa.tracks[..top] {
        if t.mute || (solo && !t.solo) {
            continue;
        }
        if let Some((si, ff)) = t.strip_at(f) {
            let s = &t.strips[si];
            let Some(act) = anim.actions.get(s.action) else { continue };
            let layer = nla::sample(act, s.action_time(ff));
            nla::blend_layer(&mut acc, &layer, s.blend, s.influence_at(ff));
        }
    }
    if oa.tweak.is_none() {
        if let Some(act) = oa.action.and_then(|a| anim.actions.get(a)) {
            let layer = nla::sample(act, f);
            if acc.is_empty() && oa.blend == Blend::Replace && oa.influence >= 1.0 {
                acc = layer;
            } else {
                nla::blend_layer(&mut acc, &layer, oa.blend, oa.influence);
            }
        }
    }
    acc
}

/// Pose the animated objects of `scene` at `frame`. Returns true when a
/// value changed.
pub fn pose_scene(anim: &Anim, scene: &mut Scene, f: f32) -> bool {
    pose_scene_full(anim, scene, f).0
}

type DriverResult = (usize, usize, f32, Option<String>);

fn pose_scene_full(anim: &Anim, scene: &mut Scene, f: f32) -> (bool, Vec<DriverResult>) {
    let mut changed = false;
    for oa in &anim.objects {
        let Some(oi) = scene.find_object(&oa.object) else { continue };
        let ch = channels(anim, oa, f);
        let mut by_path: BTreeMap<&str, Vec<(usize, f32)>> = BTreeMap::new();
        for ((p, i), v) in &ch {
            by_path.entry(p.as_str()).or_default().push((*i, *v));
        }
        for (p, vals) in by_path {
            let Some(mut arr) = path::get_array(scene, oi, p) else { continue };
            let before = arr.clone();
            for (i, v) in vals {
                if let Some(x) = arr.get_mut(i) {
                    *x = v;
                }
            }
            if arr != before && path::set_array(scene, oi, p, &arr) {
                changed = true;
            }
        }
    }
    let mut results = Vec::new();
    for (o, oa) in anim.objects.iter().enumerate() {
        let Some(oi) = scene.find_object(&oa.object) else { continue };
        for (di, d) in oa.drivers.iter().enumerate() {
            if d.mute {
                continue;
            }
            let mut d = d.clone();
            match driver::evaluate(scene, oi, &mut d, f) {
                Some(v) => {
                    if path::get(scene, oi, &d.path, d.index).is_some_and(|old| old != v) && path::set(scene, oi, &d.path, d.index, v) {
                        changed = true;
                    }
                    results.push((o, di, v, d.error.clone()));
                }
                None => results.push((o, di, d.value, d.error.clone())),
            }
        }
    }
    (changed, results)
}

fn snapshot(doc: &Doc) -> Vec<(String, Transform)> {
    doc.scene.selected.iter().filter_map(|&i| doc.scene.objects.get(i)).map(|o| (o.name.clone(), o.transform)).collect()
}

fn has_transform_driver(anim: &Anim, name: &str) -> bool {
    anim.get(name).is_some_and(|o| o.drivers.iter().any(|d| matches!(d.path.as_str(), "location" | "rotation_euler" | "rotation_quaternion" | "rotation_axis_angle" | "scale")))
}

/// Per-frame bookkeeping, run before each frame is drawn and after its
/// operators: reverse playback, posing objects when the frame changed,
/// drivers, auto-keying and logging reports for the Info editor. Returns
/// true when something visible changed.
pub fn tick(doc: &mut Doc) -> bool {
    let mut changed = false;
    if let Some(last) = doc.anim.last_frame {
        let (s, e) = (doc.scene.frame_start, doc.scene.frame_end);
        if doc.playing && doc.anim.reverse && (doc.frame == last + 1 || (doc.frame == s && last >= e)) {
            doc.frame = if last <= s { e } else { last - 1 };
        }
    }
    if doc.scene.frame_current != doc.frame {
        // Modifiers that animate (Wave, Build) evaluate at the scene's frame.
        doc.scene.frame_current = doc.frame;
        if doc.scene.objects.iter().any(|o| o.modifiers.iter().any(|m| m.is_animated())) {
            doc.touch();
            changed = true;
        }
    }
    let has_drivers = doc.anim.objects.iter().any(|o| !o.drivers.is_empty());
    if doc.anim.last_frame != Some(doc.frame) || doc.anim.dirty {
        changed |= doc.anim.last_frame.is_some_and(|l| l != doc.frame);
        changed |= evaluate(doc);
        changed |= doc.anim.dirty;
        doc.anim.last_frame = Some(doc.frame);
        doc.anim.dirty = false;
        doc.anim.seen = snapshot(doc);
    } else {
        if has_drivers {
            changed |= evaluate(doc);
        }
        if doc.anim.auto_key && !doc.playing && doc.mode != Mode::Pose {
            let now = snapshot(doc);
            if now != doc.anim.seen {
                let moved: Vec<usize> =
                    now.iter().filter(|n| !doc.anim.seen.contains(n) && !has_transform_driver(&doc.anim, &n.0)).filter_map(|n| doc.scene.find_object(&n.0)).collect();
                if !moved.is_empty() {
                    autokey_objects(doc, &moved);
                    changed = true;
                }
                doc.anim.seen = now;
            }
        }
    }
    // Simulations step towards the new frame (a few frames per tick).
    changed |= crate::physics::step(doc);
    if doc.report != doc.info.last {
        doc.info.last = doc.report.clone();
        if !doc.report.is_empty() {
            let kind = classify(&doc.report);
            doc.info.reports.push(Report { kind, text: doc.report.clone() });
            changed = true;
        }
    }
    changed
}

fn classify(s: &str) -> ReportKind {
    let l = s.to_ascii_lowercase();
    if l.contains("error") || l.contains("cannot") || l.contains("failed") || l.contains("line ") && l.contains(':') {
        ReportKind::Error
    } else if l.contains("not available") || l.contains("nothing") || l.contains("no ") {
        ReportKind::Warning
    } else {
        ReportKind::Info
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    Info,
    Warning,
    Error,
    /// Text a script printed.
    Output,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub kind: ReportKind,
    pub text: String,
}

/// Report history, the shared clipboard and the editor mailbox.
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub reports: Vec<Report>,
    last: String,
    pub clipboard: String,
    /// Operators addressed to one editor (`arg` = "@ID:rest"), drained by it.
    pub mailbox: Vec<Op>,
}

/// Report to the status bar and the Info log with an explicit kind.
pub fn report(doc: &mut Doc, kind: ReportKind, text: &str) {
    let line = text.lines().last().unwrap_or("").to_string();
    doc.report = line.clone();
    doc.info.last = line;
    for l in text.lines() {
        doc.info.reports.push(Report { kind, text: l.to_string() });
    }
}

/// A fresh id for an editor instance, used to address menu operators to it.
pub fn editor_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Take the mailbox operators addressed to editor `id`, as (op id, rest of arg).
pub fn take_mail(doc: &mut Doc, id: u64) -> Vec<(String, String)> {
    let tag = format!("@{id}:");
    let mut out = Vec::new();
    doc.info.mailbox.retain(|op| match op.arg.strip_prefix(&tag) {
        Some(rest) => {
            out.push((op.id.clone(), rest.to_string()));
            false
        }
        None => true,
    });
    out
}

/// An operator addressed to editor `id`.
pub fn mail(op: &str, id: u64, arg: &str) -> Op {
    Op::with(op, &format!("@{id}:{arg}"))
}

fn selected_names(doc: &Doc) -> Vec<String> {
    doc.scene.selected.iter().filter_map(|&i| doc.scene.objects.get(i)).map(|o| o.name.clone()).collect()
}

/// Frames that carry keys on the selected objects, sorted and unique.
pub fn selected_key_frames(doc: &Doc) -> Vec<f32> {
    let mut f: Vec<f32> = selected_names(doc).iter().flat_map(|n| doc.anim.key_frames(n)).collect();
    if doc.mode == Mode::Pose {
        if let Some(oi) = crate::rig::pose_object(doc) {
            f.extend(doc.anim.key_frames(&doc.scene.objects[oi].name));
        }
    }
    f.sort_by(f32::total_cmp);
    f.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    f
}

/// Delete every selected key; curves left empty go away.
pub fn delete_selected_keys(doc: &mut Doc) -> usize {
    let mut n = 0;
    for act in &mut doc.anim.actions {
        for c in &mut act.curves {
            let before = c.keys.len();
            c.keys.retain(|k| !k.select);
            n += before - c.keys.len();
            c.recalc_handles();
        }
        act.curves.retain(|c| !c.keys.is_empty());
    }
    doc.anim.gc();
    doc.anim.dirty = true;
    n
}

/// Change every selected key with `f`, then recompute handles.
fn for_selected_keys(doc: &mut Doc, f: impl Fn(&mut Keyframe)) {
    for act in &mut doc.anim.actions {
        for c in &mut act.curves {
            let mut any = false;
            for k in c.keys.iter_mut().filter(|k| k.select) {
                f(k);
                any = true;
            }
            if any {
                c.recalc_handles();
            }
        }
    }
    doc.anim.dirty = true;
}

/// Curves that hold a selected key, or are selected.
fn selected_curves(doc: &Doc) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (a, act) in doc.anim.actions.iter().enumerate() {
        for (c, fc) in act.curves.iter().enumerate() {
            if fc.select || fc.keys.iter().any(|k| k.select) {
                out.push((a, c));
            }
        }
    }
    out
}

/// Shift+D in the animation editors: copies of the selected keys, selected
/// in place of the originals (the editor then moves them).
pub fn duplicate_selected_keys(doc: &mut Doc) -> usize {
    let mut n = 0;
    for act in &mut doc.anim.actions {
        for c in &mut act.curves {
            let sel: Vec<Keyframe> = c.keys.iter().filter(|k| k.select).copied().collect();
            for k in &mut c.keys {
                k.select = false;
                k.select_left = false;
                k.select_right = false;
            }
            for mut k in sel {
                // Nudge so the copy is its own key until moved; sort merges back.
                k.frame += 1e-3;
                k.left[0] += 1e-3;
                k.right[0] += 1e-3;
                c.keys.push(k);
                n += 1;
            }
            c.keys.sort_by(|a, b| a.frame.total_cmp(&b.frame));
        }
    }
    doc.anim.dirty = true;
    n
}

/// Animation operators. Returns false for names it does not know.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    if arg.starts_with('@') {
        doc.info.mailbox.push(op.clone());
        return true;
    }
    let frame = doc.frame as f32;
    match op.id.as_str() {
        "anim.keyframe_insert" | "anim.keyframe_insert_by_name" | "action.keyframe_insert" | "graph.keyframe_insert" => {
            let id = if arg.is_empty() { doc.anim.keying_set.clone().unwrap_or_default() } else { arg.to_string() };
            doc.push_undo("Insert Keyframes");
            let n = insert_keying_set(doc, frame, &id);
            doc.anim.dirty = true;
            doc.report = if n == 0 {
                "Nothing selected to insert keyframes on".into()
            } else if doc.mode == Mode::Pose {
                format!("Inserted keyframes on {n} bone(s) at frame {}", doc.frame)
            } else {
                format!("Inserted keyframes on {n} object(s) at frame {}", doc.frame)
            };
        }
        "anim.keying_set_active_set" => {
            doc.anim.keying_set = if arg.is_empty() || arg == "NONE" { None } else { Some(arg.to_string()) };
            doc.report = match &doc.anim.keying_set {
                Some(k) => format!("Active keying set: {}", keying_set_name(k)),
                None => "No active keying set".into(),
            };
        }
        "anim.keyframe_delete" | "anim.keyframe_delete_v3d" => {
            doc.push_undo("Delete Keyframes");
            let n = delete_keys_at(doc, frame);
            doc.report = if n == 0 { "No keyframes to delete on this frame".into() } else { format!("Deleted {n} keyframe(s)") };
        }
        "anim.keyframes_clear" | "anim.keyframe_clear_v3d" => {
            doc.push_undo("Clear Keyframes");
            let mut n = 0;
            if doc.mode == Mode::Pose {
                if let Some(oi) = crate::rig::pose_object(doc) {
                    let name = doc.scene.objects[oi].name.clone();
                    let bones = crate::rig::selected_pose_bones(doc, oi);
                    if let Some(a) = doc.anim.edit_action(&name) {
                        let act = &mut doc.anim.actions[a];
                        let before = act.curves.len();
                        act.curves.retain(|c| path::bone_of(&c.path).is_none_or(|b| !bones.contains(&b)));
                        n = before - act.curves.len();
                    }
                }
            } else {
                for name in selected_names(doc) {
                    if let Some(oa) = doc.anim.get_mut(&name) {
                        n += 1;
                        oa.action = None;
                        oa.tweak = None;
                    }
                }
            }
            doc.anim.gc();
            doc.anim.dirty = true;
            doc.report = format!("Cleared animation on {n} channel owner(s)");
        }
        // I / Alt+I / Shift+Alt+I over a property field: "object\tpath\tindex" (-1 all).
        "anim.keyframe_insert_button" | "anim.keyframe_delete_button" | "anim.keyframe_clear_button" => {
            let Some((oi, p, idx)) = parse_button(doc, arg) else { return true };
            let name = doc.scene.objects[oi].name.clone();
            match op.id.as_str() {
                "anim.keyframe_insert_button" => {
                    doc.push_undo("Insert Keyframe");
                    let n = insert_path(doc, oi, &p, idx, frame);
                    doc.report = if n == 0 { format!("Could not insert keyframe on '{p}'") } else { format!("Inserted {n} keyframe(s) on {}", path::channel_label(&p, idx.unwrap_or(0), 1)) };
                }
                "anim.keyframe_delete_button" => {
                    doc.push_undo("Delete Keyframe");
                    let (t, a) = match doc.anim.get(&name) {
                        Some(oa) => (oa.action_time(frame), oa.edit_action()),
                        None => (frame, None),
                    };
                    let mut n = 0;
                    if let Some(act) = a.and_then(|a| doc.anim.actions.get_mut(a)) {
                        for c in act.curves.iter_mut().filter(|c| c.path == p && idx.is_none_or(|i| i == c.index)) {
                            let before = c.keys.len();
                            c.keys.retain(|k| (k.frame - t).abs() >= 0.01);
                            n += before - c.keys.len();
                            c.recalc_handles();
                        }
                        act.curves.retain(|c| !c.keys.is_empty());
                    }
                    doc.anim.gc();
                    doc.report = if n == 0 { "No keyframe on this frame".into() } else { format!("Deleted {n} keyframe(s)") };
                }
                _ => {
                    doc.push_undo("Clear Keyframes");
                    if let Some(act) = doc.anim.edit_action(&name).and_then(|a| doc.anim.actions.get_mut(a)) {
                        act.curves.retain(|c| !(c.path == p && idx.is_none_or(|i| i == c.index)));
                    }
                    doc.anim.gc();
                }
            }
            doc.anim.dirty = true;
        }
        "anim.driver_button_add" | "anim.driver_button_remove" => {
            let Some((oi, p, idx)) = parse_button(doc, arg) else { return true };
            let name = doc.scene.objects[oi].name.clone();
            let len = path::get_array(&doc.scene, oi, &p).map_or(1, |v| v.len());
            let indices: Vec<usize> = match idx {
                Some(i) => vec![i],
                None => (0..len).collect(),
            };
            if op.id == "anim.driver_button_add" {
                doc.push_undo("Add Driver");
                let o = doc.anim.entry(&name);
                let mut n = 0;
                for i in indices {
                    if doc.anim.objects[o].drivers.iter().any(|d| d.path == p && d.index == i) {
                        continue;
                    }
                    let mut d = Driver::new(&p, i);
                    // Blender seeds the variable with the current value's owner.
                    d.vars[0].targets[0].object = name.clone();
                    doc.anim.objects[o].drivers.push(d);
                    n += 1;
                }
                doc.report = format!("Added {n} driver(s)");
            } else {
                doc.push_undo("Remove Driver");
                if let Some(oa) = doc.anim.get_mut(&name) {
                    oa.drivers.retain(|d| !(d.path == p && indices.contains(&d.index)));
                }
                doc.anim.gc();
            }
            doc.anim.dirty = true;
        }
        "screen.keyframe_jump" => {
            let frames = selected_key_frames(doc);
            let to = if arg == "PREV" { frames.iter().rev().find(|&&k| k < frame - 0.01) } else { frames.iter().find(|&&k| k > frame + 0.01) };
            match to {
                Some(&k) => doc.frame = k.round() as i32,
                None => doc.report = "No more keyframes to jump to in this direction".into(),
            }
        }
        "anim.change_frame" => {
            if let Ok(f) = arg.parse::<f32>() {
                doc.frame = f.round() as i32;
            }
        }
        "anim.default_interpolation" => {
            if let Some(i) = Interp::from_arg(arg) {
                doc.anim.interp = i;
            }
        }
        "anim.default_key_type" => {
            if let Some(k) = KeyType::from_arg(arg) {
                doc.anim.key_type = k;
            }
        }
        "anim.default_handle_type" => {
            if let Some(h) = HandleType::from_arg(arg) {
                doc.anim.handle = h;
            }
        }
        "anim.auto_keying_toggle" => doc.anim.auto_key = !doc.anim.auto_key,
        "anim.start_frame_set" => {
            doc.push_undo("Set Start Frame");
            doc.scene.frame_start = doc.frame.min(doc.scene.frame_end)
        }
        "anim.end_frame_set" => {
            doc.push_undo("Set End Frame");
            doc.scene.frame_end = doc.frame.max(doc.scene.frame_start)
        }
        "screen.animation_play_reverse" => {
            doc.anim.reverse = true;
            doc.playing = !doc.playing;
        }
        "action.delete" | "graph.delete" => {
            doc.push_undo("Delete Keyframes");
            let n = delete_selected_keys(doc);
            doc.report = format!("Deleted {n} keyframe(s)");
        }
        "action.select_all" | "graph.select_all" => {
            let all = doc.anim.any_selected_key();
            for k in doc.anim.keys_mut() {
                let v = match arg {
                    "DESELECT" => false,
                    "INVERT" => !k.select,
                    "TOGGLE" => !all,
                    _ => true,
                };
                k.select = v;
                k.select_left = v && arg != "INVERT";
                k.select_right = v && arg != "INVERT";
                if arg == "DESELECT" || arg == "INVERT" || (arg == "TOGGLE" && all) {
                    k.select_left = false;
                    k.select_right = false;
                }
            }
        }
        "action.interpolation_type" | "graph.interpolation_type" => {
            let Some(i) = Interp::from_arg(arg) else { return true };
            doc.push_undo("Set Keyframe Interpolation");
            for_selected_keys(doc, |k| k.interp = i);
        }
        "action.easing_type" | "graph.easing_type" => {
            let Some(e) = Easing::from_arg(arg) else { return true };
            doc.push_undo("Set Keyframe Easing");
            for_selected_keys(doc, |k| k.easing = e);
        }
        "action.handle_type" | "graph.handle_type" => {
            let Some(h) = HandleType::from_arg(arg) else { return true };
            doc.push_undo("Set Keyframe Handle Type");
            for_selected_keys(doc, |k| {
                k.h1 = h;
                k.h2 = h;
            });
        }
        "action.keyframe_type" | "graph.keyframe_type" => {
            let Some(t) = KeyType::from_arg(arg) else { return true };
            doc.push_undo("Set Keyframe Type");
            for_selected_keys(doc, |k| k.key_type = t);
        }
        "action.extrapolation_type" | "graph.extrapolation_type" => {
            doc.push_undo("Set F-Curve Extrapolation");
            for (a, c) in selected_curves(doc) {
                let fc = &mut doc.anim.actions[a].curves[c];
                match arg {
                    "LINEAR" => fc.extrapolation = Extrapolation::Linear,
                    "MAKE_CYCLIC" => fc.set_cyclic(true),
                    "CLEAR_CYCLIC" => fc.set_cyclic(false),
                    _ => fc.extrapolation = Extrapolation::Constant,
                }
            }
            doc.anim.dirty = true;
        }
        "graph.fmodifier_add" => {
            doc.push_undo("Add F-Curve Modifier");
            let only_active = arg.ends_with(":ACTIVE");
            let ty = arg.trim_end_matches(":ACTIVE");
            let mut cs = selected_curves(doc);
            if only_active {
                cs.truncate(1);
            }
            for (a, c) in cs {
                let m = if ty == "NOISE" { FModifier::noise() } else { FModifier::cycles() };
                doc.anim.actions[a].curves[c].modifiers.push(m);
            }
            doc.anim.dirty = true;
        }
        "action.frame_jump" | "graph.frame_jump" => {
            let sel: Vec<f32> = doc.anim.selected_keys().iter().filter_map(|r| doc.anim.key(*r).map(|k| k.frame)).collect();
            if !sel.is_empty() {
                doc.frame = (sel.iter().sum::<f32>() / sel.len() as f32).round() as i32;
            }
        }
        "action.select_column" | "graph.select_column" => {
            // Columns on selected keys (K), or the current frame (Ctrl+K).
            let frames: Vec<f32> = if arg == "CFRA" { vec![frame] } else { doc.anim.selected_keys().iter().filter_map(|r| doc.anim.key(*r).map(|k| k.frame)).collect() };
            for k in doc.anim.keys_mut() {
                if frames.iter().any(|f| (k.frame - f).abs() < 0.01) {
                    k.select = true;
                }
            }
        }
        "action.snap" | "graph.snap" => {
            doc.push_undo("Snap Keys");
            let f = frame;
            for_selected_keys(doc, |k| {
                let to = if arg == "CFRA" { f } else { k.frame.round() };
                let d = to - k.frame;
                k.shift(d, 0.0);
            });
            for act in &mut doc.anim.actions {
                for c in &mut act.curves {
                    c.sort();
                }
            }
        }
        "action.mirror" | "graph.mirror" => {
            doc.push_undo("Mirror Keys");
            let f = frame;
            for_selected_keys(doc, |k| {
                if arg == "XAXIS" {
                    k.value = -k.value;
                    k.left[1] = -k.left[1];
                    k.right[1] = -k.right[1];
                } else {
                    k.frame = 2.0 * f - k.frame;
                    let (l, r) = (k.left, k.right);
                    k.left = [2.0 * f - r[0], r[1]];
                    k.right = [2.0 * f - l[0], l[1]];
                }
            });
            for act in &mut doc.anim.actions {
                for c in &mut act.curves {
                    c.sort();
                }
            }
        }
        "action.clean" | "graph.clean" => {
            doc.push_undo("Clean Keyframes");
            let mut n = 0;
            for (a, c) in selected_curves(doc) {
                let fc = &mut doc.anim.actions[a].curves[c];
                let before = fc.keys.len();
                let mut keep: Vec<Keyframe> = Vec::new();
                for (i, k) in fc.keys.iter().enumerate() {
                    let prev = keep.last();
                    let next = fc.keys.get(i + 1);
                    let redundant = match (prev, next) {
                        (Some(p), Some(q)) => (p.value - k.value).abs() < 1e-3 && (q.value - k.value).abs() < 1e-3,
                        (Some(p), None) => (p.value - k.value).abs() < 1e-3,
                        _ => false,
                    };
                    if !redundant || !k.select {
                        keep.push(*k);
                    }
                }
                fc.keys = keep;
                fc.recalc_handles();
                n += before - fc.keys.len();
            }
            doc.anim.dirty = true;
            doc.report = format!("Cleaned {n} keyframe(s)");
        }
        "action.duplicate" | "graph.duplicate" => {
            doc.push_undo("Duplicate Keyframes");
            duplicate_selected_keys(doc);
        }
        "action.new" => {
            let Some(a) = doc.active() else { return true };
            doc.push_undo("New Action");
            let name = doc.scene.objects[a].name.clone();
            let o = doc.anim.entry(&name);
            let base = doc.anim.objects[o].action.map(|x| doc.anim.actions[x].clone());
            let mut act = base.unwrap_or_else(|| Action::new(&format!("{name}Action")));
            act.name = sex_3d::scene::unique_name(&act.name, |n| doc.anim.actions.iter().any(|a| a.name == n));
            doc.anim.actions.push(act);
            doc.anim.objects[o].action = Some(doc.anim.actions.len() - 1);
            doc.anim.dirty = true;
        }
        "action.unlink" => {
            let Some(a) = doc.active() else { return true };
            doc.push_undo("Unlink Action");
            let name = doc.scene.objects[a].name.clone();
            if let Some(oa) = doc.anim.get_mut(&name) {
                oa.action = None;
            }
            doc.anim.gc();
            doc.anim.dirty = true;
        }
        "action.assign" => {
            // arg: action name
            let Some(a) = doc.active() else { return true };
            let Some(ai) = doc.anim.actions.iter().position(|x| x.name == arg) else { return true };
            doc.push_undo("Assign Action");
            let name = doc.scene.objects[a].name.clone();
            let o = doc.anim.entry(&name);
            doc.anim.objects[o].action = Some(ai);
            doc.anim.gc();
            doc.anim.dirty = true;
        }
        "action.push_down" | "nla.action_pushdown" => {
            let name = if arg.is_empty() { doc.active().map(|a| doc.scene.objects[a].name.clone()) } else { Some(arg.to_string()) };
            let Some(name) = name else { return true };
            if !push_down(doc, &name) {
                doc.report = "No action to push down".into();
            }
        }
        "action.stash" => {
            let Some(a) = doc.active() else { return true };
            let name = doc.scene.objects[a].name.clone();
            if push_down(doc, &name) {
                if let Some(oa) = doc.anim.get_mut(&name) {
                    if let Some(t) = oa.tracks.last_mut() {
                        t.name = "[Action Stash]".into();
                        t.mute = true;
                    }
                }
            }
        }
        _ if op.id.starts_with("nla.") => return nla_op(doc, op),
        _ if props::run(doc, op) => {}
        "marker.add" | "marker.move" => {
            doc.report = format!("{} is not available yet", op.id);
        }
        _ => return false,
    }
    true
}

/// "object\tpath\tindex" (index -1 for every element).
fn parse_button(doc: &Doc, arg: &str) -> Option<(usize, String, Option<usize>)> {
    let mut it = arg.split('\t');
    let obj = it.next()?;
    let p = it.next()?.to_string();
    let idx: i64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(-1);
    let oi = doc.scene.find_object(obj)?;
    Some((oi, p, (idx >= 0).then_some(idx as usize)))
}

/// Push Down: the active action becomes a strip on a new track at the top
/// of the NLA stack.
pub fn push_down(doc: &mut Doc, name: &str) -> bool {
    let Some(o) = doc.anim.objects.iter().position(|x| x.object == name) else { return false };
    let Some(a) = doc.anim.objects[o].action else { return false };
    if doc.anim.actions[a].curves.is_empty() {
        return false;
    }
    doc.push_undo("Push Down Action");
    let act = &doc.anim.actions[a];
    let (start, _) = act.frame_range();
    let strip = NlaStrip::new(&act.name, a, act, start);
    let oa = &mut doc.anim.objects[o];
    let mut t = NlaTrack::new(&unique_track_name(&oa.tracks));
    t.strips.push(strip);
    oa.tracks.push(t);
    oa.action = None;
    doc.anim.dirty = true;
    doc.report = "Pushed down action".into();
    true
}

fn unique_track_name(tracks: &[NlaTrack]) -> String {
    sex_3d::scene::unique_name("NlaTrack", |n| tracks.iter().any(|t| t.name == n))
}

/// The NLA editor's operators. Strips are addressed by selection (and
/// `arg` "object" for the active-object ones).
fn nla_op(doc: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    match op.id.as_str() {
        "nla.tracks_add" => {
            let Some(a) = doc.active() else { return true };
            doc.push_undo("Add Tracks");
            let name = doc.scene.objects[a].name.clone();
            let o = doc.anim.entry(&name);
            let t = NlaTrack::new(&unique_track_name(&doc.anim.objects[o].tracks));
            doc.anim.objects[o].tracks.push(t);
        }
        "nla.actionclip_add" => {
            // arg: action name; added to the active object's top track at the frame.
            let Some(a) = doc.active() else { return true };
            let Some(ai) = doc.anim.actions.iter().position(|x| x.name == arg).or_else(|| (!doc.anim.actions.is_empty()).then_some(0)) else {
                doc.report = "No actions to add".into();
                return true;
            };
            doc.push_undo("Add Action Strip");
            let name = doc.scene.objects[a].name.clone();
            let o = doc.anim.entry(&name);
            let act = &doc.anim.actions[ai];
            let strip = NlaStrip::new(&act.name, ai, act, doc.frame as f32);
            let oa = &mut doc.anim.objects[o];
            let fits = oa.tracks.last().is_some_and(|t| t.has_space(strip.frame_start, strip.frame_end));
            if !fits {
                let t = NlaTrack::new(&unique_track_name(&oa.tracks));
                oa.tracks.push(t);
            }
            let t = oa.tracks.last_mut().expect("track");
            t.strips.push(strip);
            t.sort();
            doc.anim.dirty = true;
        }
        "nla.tweakmode_enter" | "nla.tweakmode_exit" | "nla.tweakmode_toggle" => {
            let entering = match op.id.as_str() {
                "nla.tweakmode_enter" => true,
                "nla.tweakmode_exit" => false,
                _ => !doc.anim.objects.iter().any(|o| o.tweak.is_some()),
            };
            if entering {
                let mut n = 0;
                for oa in &mut doc.anim.objects {
                    let found = oa.tracks.iter().enumerate().rev().find_map(|(ti, t)| t.strips.iter().position(|s| s.select).map(|si| (ti, si)));
                    if let Some(ts) = found {
                        oa.tweak = Some(ts);
                        n += 1;
                    }
                }
                doc.report = if n == 0 { "No active strip(s) to enter tweak mode on".into() } else { "Tweak mode: editing the strip's action".into() };
            } else {
                for oa in &mut doc.anim.objects {
                    if let Some((t, s)) = oa.tweak.take() {
                        // Keys may have moved: the strip follows its action's range.
                        let range = oa.tracks.get(t).and_then(|t| t.strips.get(s)).and_then(|s| doc.anim.actions.get(s.action)).map(|a| a.frame_range());
                        if let (Some((a, b)), Some(st)) = (range, oa.tracks.get_mut(t).and_then(|t| t.strips.get_mut(s))) {
                            st.action_start = a;
                            st.action_end = b;
                            st.update_end();
                        }
                    }
                }
            }
            doc.anim.dirty = true;
        }
        "nla.delete" => {
            doc.push_undo("Delete Strips");
            for oa in &mut doc.anim.objects {
                oa.tweak = None;
                for t in &mut oa.tracks {
                    t.strips.retain(|s| !s.select);
                }
                oa.tracks.retain(|t| !t.select);
            }
            doc.anim.gc();
            doc.anim.dirty = true;
        }
        "nla.duplicate" => {
            doc.push_undo("Duplicate Strips");
            for oa in &mut doc.anim.objects {
                let mut add: Vec<(usize, NlaStrip)> = Vec::new();
                for (ti, t) in oa.tracks.iter_mut().enumerate() {
                    for s in &mut t.strips {
                        if s.select {
                            let mut c = s.clone();
                            s.select = false;
                            let len = c.frame_end - c.frame_start;
                            c.frame_start = c.frame_end;
                            c.frame_end = c.frame_start + len;
                            add.push((ti, c));
                        }
                    }
                }
                for (ti, s) in add {
                    let fits = oa.tracks[ti].has_space(s.frame_start, s.frame_end);
                    let ti = if fits {
                        ti
                    } else {
                        let t = NlaTrack::new(&unique_track_name(&oa.tracks));
                        oa.tracks.insert(ti + 1, t);
                        ti + 1
                    };
                    oa.tracks[ti].strips.push(s);
                    oa.tracks[ti].sort();
                }
            }
            doc.anim.dirty = true;
        }
        "nla.mute_toggle" => {
            doc.push_undo("Toggle Muting");
            for oa in &mut doc.anim.objects {
                for t in &mut oa.tracks {
                    for s in t.strips.iter_mut().filter(|s| s.select) {
                        s.mute = !s.mute;
                    }
                }
            }
            doc.anim.dirty = true;
        }
        "nla.split" => {
            doc.push_undo("Split Strips");
            let f = doc.frame as f32;
            for oa in &mut doc.anim.objects {
                for t in &mut oa.tracks {
                    let mut add = Vec::new();
                    for s in t.strips.iter_mut().filter(|s| s.select && f > s.frame_start && f < s.frame_end && s.repeat <= 1.0) {
                        let mid = s.action_time(f);
                        let mut b = s.clone();
                        s.action_end = mid;
                        s.update_end();
                        b.action_start = mid;
                        b.frame_start = s.frame_end;
                        b.update_end();
                        b.select = false;
                        add.push(b);
                    }
                    t.strips.extend(add);
                    t.sort();
                }
            }
            doc.anim.dirty = true;
        }
        "nla.select_all" => {
            let any = doc.anim.objects.iter().any(|o| o.tracks.iter().any(|t| t.strips.iter().any(|s| s.select)));
            for oa in &mut doc.anim.objects {
                for t in &mut oa.tracks {
                    for s in &mut t.strips {
                        s.select = match arg {
                            "DESELECT" => false,
                            "INVERT" => !s.select,
                            "TOGGLE" => !any,
                            _ => true,
                        };
                    }
                }
            }
        }
        "nla.set_prop" => {
            // "key=value" on every selected strip.
            let Some((k, v)) = arg.split_once('=') else { return true };
            doc.push_undo("Strip Property");
            for oa in &mut doc.anim.objects {
                for t in &mut oa.tracks {
                    for s in t.strips.iter_mut().filter(|s| s.select) {
                        match k {
                            "blend_type" => s.blend = Blend::from_arg(v).unwrap_or(s.blend),
                            "extrapolation" => s.extrapolation = StripExtrap::from_arg(v).unwrap_or(s.extrapolation),
                            "use_reverse" => s.reversed = v == "1" || v == "true",
                            "mute" => s.mute = v == "1" || v == "true",
                            _ => {}
                        }
                    }
                }
            }
            doc.anim.dirty = true;
        }
        _ => {
            doc.report = format!("{} is not available yet", op.id);
        }
    }
    true
}

/// Take over the animation of an imported .blend: every curve keeps its data
/// path; objects with a parent inverse matrix get their transform curves
/// baked through it at every key frame, so the evaluated pose matches
/// Blender's.
pub fn adopt_blend_actions(doc: &mut Doc, actions: &[sex_3d::io::blend::BlendAction]) {
    use sex_3d::io::blend::{INTERP_CONSTANT, INTERP_LINEAR};
    use sex_3d::math::Mat4;
    use sex_3d::Quat;
    const TRS: [&str; 3] = ["location", "rotation_euler", "scale"];
    for a in actions {
        let Some(oi) = doc.scene.find_object(&a.object) else { continue };
        let mut raw = Action::new(&a.action);
        for c in &a.curves {
            let curve = raw.curve_mut(&c.path, c.index);
            for k in &c.keys {
                let interp = match k.interpolation {
                    INTERP_CONSTANT => Interp::Constant,
                    INTERP_LINEAR => Interp::Linear,
                    _ => Interp::Bezier,
                };
                curve.insert(k.frame, k.value, interp);
            }
        }
        let mut out = raw.clone();
        if let Some(inv) = a.parent_inverse.map(Mat4::from_cols_array) {
            // The scene transform is inv × raw; recover raw's unanimated channels.
            let base = Transform::from_matrix(&(inv.inverse() * doc.scene.objects[oi].transform.matrix()));
            let frames = raw.key_frames();
            let interp_at = |f: f32| raw.curves.iter().flat_map(|c| c.keys.iter()).find(|k| (k.frame - f).abs() < 0.01).map_or(Interp::Bezier, |k| k.interp);
            out.curves.retain(|c| !TRS.contains(&c.path.as_str()));
            let e0 = sex_3d::script::quat_to_euler_xyz(base.rotation);
            for &f in &frames {
                let val = |p: &str, i: usize, d: f32| raw.curves.iter().find(|c| c.path == p && c.index == i && !c.keys.is_empty()).map_or(d, |c| c.evaluate(f));
                let mut t = base;
                t.location = sex_3d::Vec3::new(val("location", 0, base.location.x), val("location", 1, base.location.y), val("location", 2, base.location.z));
                if raw.curves.iter().any(|c| c.path == "rotation_euler") {
                    t.rotation = Quat::from_euler_xyz(val("rotation_euler", 0, e0[0]), val("rotation_euler", 1, e0[1]), val("rotation_euler", 2, e0[2]));
                }
                t.scale = sex_3d::Vec3::new(val("scale", 0, base.scale.x), val("scale", 1, base.scale.y), val("scale", 2, base.scale.z));
                let baked = Transform::from_matrix(&(inv * t.matrix()));
                let e = sex_3d::script::quat_to_euler_xyz(baked.rotation);
                let vals = [baked.location.to_array(), e, baked.scale.to_array()];
                for (pi, p) in TRS.iter().enumerate() {
                    for i in 0..3 {
                        out.curve_mut(p, i).insert(f, vals[pi][i], interp_at(f));
                    }
                }
            }
        }
        let name = doc.scene.objects[oi].name.clone();
        out.name = sex_3d::scene::unique_name(&out.name, |n| doc.anim.actions.iter().any(|x| x.name == n));
        doc.anim.actions.push(out);
        let ai = doc.anim.actions.len() - 1;
        let o = doc.anim.entry(&name);
        doc.anim.objects[o].action = Some(ai);
    }
    doc.anim.gc();
    doc.anim.dirty = true;
}

/// Whether `oi` is a mesh with shape keys (the Shape Key Editor's object).
pub fn has_shape_keys(scene: &Scene, oi: usize) -> bool {
    matches!(scene.objects.get(oi).map(|o| &o.data), Some(ObjectData::Mesh(mi)) if scene.meshes.get(*mi).is_some_and(|m| m.shape_keys.len() > 1))
}

#[cfg(test)]
mod tests;
