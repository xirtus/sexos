// SPDX-License-Identifier: GPL-3.0-only
//! polyops commands behind the live `bpy` API: JSON reads of whole
//! data-blocks, operators under Blender's idnames, edit-mode selection,
//! actions and F-Curves, render/scene settings, and the registry of classes
//! a script registered (operators, panels, menus).
//!
//! Every read answers `ok =json {...}` (a bare token never starts with `=`, so
//! the Python side tells JSON from tokens by that mark). Reads never change
//! the scene, so the Python side caches them until its next write.
//!
//! The state that Blender keeps in places Polyamour's [`Scene`] has no field
//! for (the object mode, mesh element selection, actions, render settings,
//! rotation modes, registered classes) lives in [`State`], owned by the
//! [`Session`]. A host (the app) copies its own versions in and out around
//! each call (see `script::live`).
use super::*;
use crate::armature::Bone;
use crate::scene::ArmatureWeights;
use crate::math::{EulerOrder, Mat4};
use crate::ops2::Selection;
use serde_json::{json, Value as J};
use std::collections::BTreeSet;

/// Commands handled here.
pub(super) const COMMANDS: &[&str] = &[
    "ids",
    "dump_object",
    "dump_mesh",
    "dump_material",
    "dump_tree",
    "dump_scene",
    "dump_armature",
    "dump_lattice",
    "dump_image",
    "dump_action",
    "eval_mesh",
    "op",
    "op_poll",
    "op_list",
    "obj_set",
    "mesh_select",
    "mesh_attr",
    "mode_set",
    "prop_set",
    "frame_set",
    "keyframe",
    "keyframe_delete",
    "action_new",
    "action_assign",
    "fcurve_new",
    "fcurve_remove",
    "key_insert",
    "key_set",
    "key_remove",
    "register",
    "unregister",
    "report",
    "image_new",
    "image_pixels",
    "image_save",
    "image_load",
    "use_nodes",
    "object_copy",
    "mesh_copy",
    "material_copy",
    "remove",
    "rename",
    "modifier_move",
    "material_slot",
    "bone_remove",
    "noise",
];

/// Commands here that only read: replies may be cached by the caller.
pub const QUERIES: &[&str] = &[
    "ids", "dump_object", "dump_mesh", "dump_material", "dump_tree", "dump_scene", "dump_armature", "dump_lattice", "dump_image", "dump_action", "eval_mesh", "op_poll", "op_list", "noise",
];

/// Element selection of one mesh (Blender's select flags).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sel {
    pub verts: BTreeSet<u32>,
    pub edges: BTreeSet<(u32, u32)>,
    pub faces: BTreeSet<u32>,
}

impl Sel {
    /// Faces selected outright or with every vertex selected.
    pub fn faces(&self, m: &Mesh) -> Vec<u32> {
        (0..m.faces.len() as u32).filter(|&f| self.faces.contains(&f) || (!m.faces[f as usize].verts.is_empty() && m.faces[f as usize].verts.iter().all(|v| self.verts.contains(v)))).collect()
    }
    pub fn edges(&self, m: &Mesh) -> Vec<[u32; 2]> {
        m.edges().into_iter().filter(|e| self.edges.contains(&(e[0].min(e[1]), e[0].max(e[1]))) || (self.verts.contains(&e[0]) && self.verts.contains(&e[1]))).collect()
    }
    pub fn verts(&self, m: &Mesh) -> Vec<u32> {
        let mut v: BTreeSet<u32> = self.verts.clone();
        for &(a, b) in &self.edges {
            v.insert(a);
            v.insert(b);
        }
        for &f in &self.faces {
            if let Some(face) = m.faces.get(f as usize) {
                v.extend(face.verts.iter().copied());
            }
        }
        v.into_iter().filter(|&i| (i as usize) < m.positions.len()).collect()
    }
    fn from_selection(m: &Mesh, s: &Selection) -> Sel {
        let mut out = Sel { verts: s.verts.iter().copied().collect(), edges: s.edges.iter().map(|e| (e[0].min(e[1]), e[0].max(e[1]))).collect(), faces: s.faces.iter().copied().collect() };
        for &f in &s.faces {
            if let Some(face) = m.faces.get(f as usize) {
                out.verts.extend(face.verts.iter().copied());
            }
        }
        for e in &s.edges {
            out.verts.insert(e[0]);
            out.verts.insert(e[1]);
        }
        out
    }
    fn all(m: &Mesh) -> Sel {
        Sel {
            verts: (0..m.positions.len() as u32).collect(),
            edges: m.edges().into_iter().map(|e| (e[0].min(e[1]), e[0].max(e[1]))).collect(),
            faces: (0..m.faces.len() as u32).collect(),
        }
    }
    fn is_empty(&self) -> bool {
        self.verts.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }
}

/// One keyframe (Blender's `BezTriple`, as `keyframe_points[i]`).
#[derive(Clone, Debug, PartialEq)]
pub struct Key {
    pub co: [f32; 2],
    /// `CONSTANT`, `LINEAR` or `BEZIER` (towards the next key).
    pub interpolation: String,
    pub handle_left: [f32; 2],
    pub handle_right: [f32; 2],
    /// `AUTO_CLAMPED` keys get their handles recomputed; `FREE` keep them.
    pub handle_type: String,
    pub select: bool,
}

/// An animated channel: `data_path[array_index]`.
#[derive(Clone, Debug, PartialEq)]
pub struct FCurve {
    pub data_path: String,
    pub index: u32,
    pub group: String,
    pub keys: Vec<Key>,
    pub mute: bool,
}

impl FCurve {
    /// Blender's auto-clamped handles (smooth, flat at extremes and ends).
    pub fn recalc(&mut self) {
        self.keys.sort_by(|a, b| a.co[0].total_cmp(&b.co[0]));
        let n = self.keys.len();
        for i in 0..n {
            if self.keys[i].handle_type == "FREE" {
                continue;
            }
            let k = self.keys[i].co;
            let prev = (i > 0).then(|| self.keys[i - 1].co);
            let next = (i + 1 < n).then(|| self.keys[i + 1].co);
            let dl = prev.map_or(0.0, |p| (k[0] - p[0]) / 3.0);
            let dr = next.map_or(0.0, |q| (q[0] - k[0]) / 3.0);
            let mut slope = 0.0;
            if let (Some(p), Some(q)) = (prev, next) {
                let extreme = (k[1] >= p[1] && k[1] >= q[1]) || (k[1] <= p[1] && k[1] <= q[1]);
                if !extreme && q[0] > p[0] {
                    slope = (q[1] - p[1]) / (q[0] - p[0]);
                    let lim_l = if dl > 0.0 { (k[1] - p[1]).abs() / dl } else { f32::INFINITY };
                    let lim_r = if dr > 0.0 { (q[1] - k[1]).abs() / dr } else { f32::INFINITY };
                    let lim = lim_l.min(lim_r);
                    if slope.abs() > lim {
                        slope = lim * slope.signum();
                    }
                }
            }
            let dl = if prev.is_some() { dl } else { dr.max(1.0 / 3.0) };
            let dr = if next.is_some() { dr } else { dl.max(1.0 / 3.0) };
            self.keys[i].handle_left = [k[0] - dl, k[1] - slope * dl];
            self.keys[i].handle_right = [k[0] + dr, k[1] + slope * dr];
        }
    }

    /// Insert (or replace) a key at `frame`; returns its index.
    pub fn insert(&mut self, frame: f32, value: f32, interpolation: &str) -> usize {
        if let Some(i) = self.keys.iter().position(|k| (k.co[0] - frame).abs() < 0.01) {
            self.keys[i].co[1] = value;
            self.recalc();
            return i;
        }
        self.keys.push(Key { co: [frame, value], interpolation: interpolation.into(), handle_left: [frame - 1.0, value], handle_right: [frame + 1.0, value], handle_type: "AUTO_CLAMPED".into(), select: true });
        self.recalc();
        self.keys.iter().position(|k| (k.co[0] - frame).abs() < 0.01).unwrap_or(0)
    }

    /// The value at `frame` (constant extrapolation).
    pub fn evaluate(&self, frame: f32) -> f32 {
        let keys = &self.keys;
        let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return 0.0 };
        if frame <= first.co[0] {
            return first.co[1];
        }
        if frame >= last.co[0] {
            return last.co[1];
        }
        let i = keys.partition_point(|k| k.co[0] <= frame).saturating_sub(1);
        let (a, b) = (&keys[i], &keys[i + 1]);
        match a.interpolation.as_str() {
            "CONSTANT" => a.co[1],
            "LINEAR" => a.co[1] + (b.co[1] - a.co[1]) * (frame - a.co[0]) / (b.co[0] - a.co[0]),
            _ => {
                let x1 = a.handle_right[0].clamp(a.co[0], b.co[0]);
                let x2 = b.handle_left[0].clamp(a.co[0], b.co[0]);
                let bez = |p0: f32, p1: f32, p2: f32, p3: f32, t: f32| {
                    let u = 1.0 - t;
                    u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
                };
                let (mut lo, mut hi) = (0.0f32, 1.0f32);
                for _ in 0..32 {
                    let mid = 0.5 * (lo + hi);
                    if bez(a.co[0], x1, x2, b.co[0], mid) < frame {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                bez(a.co[1], a.handle_right[1], b.handle_left[1], b.co[1], 0.5 * (lo + hi))
            }
        }
    }
}

/// Blender's `Action`: F-Curves by data path.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Action {
    pub name: String,
    pub fcurves: Vec<FCurve>,
}

impl Action {
    pub fn fcurve_mut(&mut self, path: &str, index: u32, group: &str) -> &mut FCurve {
        let i = match self.fcurves.iter().position(|c| c.data_path == path && c.index == index) {
            Some(i) => i,
            None => {
                self.fcurves.push(FCurve { data_path: path.into(), index, group: group.into(), keys: Vec::new(), mute: false });
                self.fcurves.len() - 1
            }
        };
        &mut self.fcurves[i]
    }
    pub fn frame_range(&self) -> [f32; 2] {
        let mut r = [f32::INFINITY, f32::NEG_INFINITY];
        for k in self.fcurves.iter().flat_map(|c| &c.keys) {
            r[0] = r[0].min(k.co[0]);
            r[1] = r[1].max(k.co[0]);
        }
        if r[0] > r[1] {
            [0.0, 0.0]
        } else {
            r
        }
    }
}

/// A class a script registered (`bpy.utils.register_class`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Registered {
    /// `operator`, `panel`, `menu`, `header`, `ui_list`, `property_group`, `addon_preferences`.
    pub kind: String,
    /// `bl_idname` (or the class name when it has none).
    pub idname: String,
    /// Every other `bl_*` attribute, as given (`label`, `space_type`, `region_type`,
    /// `category`, `context`, `description`, `options`, `parent_id`, `order`...).
    pub attrs: BTreeMap<String, String>,
}

impl Registered {
    pub fn attr(&self, k: &str) -> &str {
        self.attrs.get(k).map(String::as_str).unwrap_or("")
    }
}

/// What Blender keeps outside Polyamour's scene.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    /// `OBJECT`, `EDIT`, `POSE`, `SCULPT`, `VERTEX_PAINT`, `WEIGHT_PAINT`, `TEXTURE_PAINT`.
    pub mode: String,
    /// Mesh element selection, by mesh index.
    pub sel: HashMap<usize, Sel>,
    /// Edit mode's select mode: vertex, edge, face.
    pub select_mode: [bool; 3],
    pub actions: Vec<Action>,
    /// Object name -> action name (`animation_data.action`).
    pub anim: BTreeMap<String, String>,
    /// Render, Cycles, colour-management and other settings the scene has no
    /// field for, by Python path (`render.resolution_x`, `cycles.samples`).
    pub props: BTreeMap<String, String>,
    /// Object name -> rotation mode (`XYZ` when missing).
    pub rotation_modes: BTreeMap<String, String>,
    /// The last Euler angles a script gave an object (keeps winding past pi).
    pub eulers: BTreeMap<String, [f32; 3]>,
    /// Objects hidden from renders (`hide_render`).
    pub hide_render: BTreeSet<String>,
    /// Bone selection of pose / edit mode: armature object name -> bones.
    pub bone_sel: BTreeMap<String, BTreeSet<String>>,
    pub registry: Vec<Registered>,
    /// `report` lines of this call: (type, message).
    pub reports: Vec<(String, String)>,
    pub cursor: [f32; 3],
    /// Files the script asked the host to open, save, import or export
    /// (handled here when the host does not take them first).
    pub files_written: Vec<String>,
    /// A newer scene replaced the whole document (wm.open_mainfile, read_homefile).
    pub replaced: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            mode: "OBJECT".into(),
            sel: HashMap::new(),
            select_mode: [true, false, false],
            actions: Vec::new(),
            anim: BTreeMap::new(),
            props: BTreeMap::new(),
            rotation_modes: BTreeMap::new(),
            eulers: BTreeMap::new(),
            hide_render: BTreeSet::new(),
            bone_sel: BTreeMap::new(),
            registry: Vec::new(),
            reports: Vec::new(),
            cursor: [0.0; 3],
            files_written: Vec::new(),
            replaced: false,
        }
    }
}

impl State {
    pub fn action(&self, name: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.name == name)
    }
    pub fn action_mut(&mut self, name: &str) -> Option<&mut Action> {
        self.actions.iter_mut().find(|a| a.name == name)
    }
    /// The objects' actions (`animation_data.action`) for a .blend file,
    /// with Blender's `BEZT_IPO_*` interpolation numbers and the handles.
    pub fn blend_actions(&self) -> Vec<crate::io::blend::BlendAction> {
        use crate::io::blend::{BlendAction, BlendFCurve, BlendKey};
        const IPO: [&str; 13] = ["CONSTANT", "LINEAR", "BEZIER", "BACK", "BOUNCE", "CIRC", "CUBIC", "ELASTIC", "EXPO", "QUAD", "QUART", "QUINT", "SINE"];
        self.anim
            .iter()
            .filter_map(|(object, action)| {
                let a = self.action(action)?;
                let curves = a
                    .fcurves
                    .iter()
                    .filter(|c| !c.keys.is_empty())
                    .map(|c| BlendFCurve {
                        path: c.data_path.clone(),
                        index: c.index as usize,
                        keys: c
                            .keys
                            .iter()
                            .map(|k| BlendKey { frame: k.co[0], value: k.co[1], interpolation: IPO.iter().position(|i| *i == k.interpolation).unwrap_or(2) as u8, handles: Some([k.handle_left, k.handle_right]) })
                            .collect(),
                    })
                    .collect();
                Some(BlendAction { object: object.clone(), action: a.name.clone(), curves, parent_inverse: None })
            })
            .collect()
    }
    fn prop_f(&self, k: &str, d: f32) -> f32 {
        self.props.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
    }
    fn prop_s<'a>(&'a self, k: &str, d: &'a str) -> &'a str {
        self.props.get(k).map(String::as_str).unwrap_or(d)
    }
    /// Render settings from the stored `render.*` / `cycles.*` properties.
    pub fn render_settings(&self, frame: i32) -> crate::render::RenderSettings {
        let pct = self.prop_f("render.resolution_percentage", 100.0) / 100.0;
        crate::render::RenderSettings {
            width: ((self.prop_f("render.resolution_x", 1920.0) * pct).round() as u32).max(1),
            height: ((self.prop_f("render.resolution_y", 1080.0) * pct).round() as u32).max(1),
            samples: (self.prop_f("cycles.samples", 16.0) as u32).max(1),
            max_bounces: self.prop_f("cycles.max_bounces", 4.0) as u32,
            denoise: self.prop_s("cycles.use_denoising", "0") == "1",
            seed: self.prop_f("cycles.seed", 0.0) as u32,
            film_transparent: self.prop_s("render.film_transparent", "0") == "1",
            frame,
            ..Default::default()
        }
    }
    /// The object's rotation order.
    pub fn euler_order(&self, obj: &str) -> EulerOrder {
        self.rotation_modes.get(obj).and_then(|m| EulerOrder::from_name(m)).unwrap_or_default()
    }

    /// Apply every assigned action at `frame` to the scene (object
    /// transforms and pose bones); returns true when anything changed.
    pub fn evaluate(&mut self, scene: &mut Scene, frame: f32) -> bool {
        let mut changed = false;
        let assigned: Vec<(String, String)> = self.anim.iter().map(|(o, a)| (o.clone(), a.clone())).collect();
        for (obj, act) in assigned {
            let (Some(oi), Some(a)) = (scene.find_object(&obj), self.action(&act).cloned()) else { continue };
            let order = self.euler_order(&obj);
            let o = &mut scene.objects[oi];
            let before = o.transform;
            let mut euler = self.eulers.get(&obj).copied().filter(|e| Quat::from_euler(*e, order).dot(o.transform.rotation).abs() > 0.9999).unwrap_or_else(|| o.transform.rotation.to_euler(order));
            let mut quat = o.transform.rotation;
            let (mut e_set, mut q_set) = (false, false);
            for c in a.fcurves.iter().filter(|c| !c.mute && !c.keys.is_empty()) {
                let v = c.evaluate(frame);
                let i = c.index as usize;
                if let Some(rest) = c.data_path.strip_prefix("pose.bones[\"") {
                    let Some((bone, prop)) = rest.split_once("\"].") else { continue };
                    let pb = rig::pose_bone_mut(&mut o.pose, bone);
                    match (prop, i) {
                        ("location", 0..=2) => pb.location = set_i(pb.location, i, v),
                        ("scale", 0..=2) => pb.scale = set_i(pb.scale, i, v),
                        ("rotation_euler", 0..=2) => pb.rotation_euler = set_i(pb.rotation_euler, i, v),
                        ("rotation_quaternion", 0..=3) => {
                            let q = &mut pb.rotation_quaternion;
                            match i {
                                0 => q.w = v,
                                1 => q.x = v,
                                2 => q.y = v,
                                _ => q.z = v,
                            }
                        }
                        ("rotation_axis_angle", 0..=3) => pb.rotation_axis_angle[i] = v,
                        _ => {}
                    }
                    changed = true;
                    continue;
                }
                match (c.data_path.as_str(), i) {
                    ("location", 0..=2) => o.transform.location = set_i(o.transform.location, i, v),
                    ("scale", 0..=2) => o.transform.scale = set_i(o.transform.scale, i, v),
                    ("rotation_euler", 0..=2) => {
                        euler[i] = v;
                        e_set = true;
                    }
                    ("rotation_quaternion", 0..=3) => {
                        match i {
                            0 => quat.w = v,
                            1 => quat.x = v,
                            2 => quat.y = v,
                            _ => quat.z = v,
                        }
                        q_set = true;
                    }
                    _ => {}
                }
            }
            let mode = self.rotation_modes.get(&obj).map(String::as_str).unwrap_or("XYZ");
            if q_set && mode == "QUATERNION" {
                o.transform.rotation = quat.normalize();
            } else if e_set {
                o.transform.rotation = Quat::from_euler(euler, order);
                self.eulers.insert(obj.clone(), euler);
            }
            changed |= o.transform != before;
        }
        changed
    }
}

fn set_i(mut v: Vec3, i: usize, x: f32) -> Vec3 {
    match i {
        0 => v.x = x,
        1 => v.y = x,
        _ => v.z = x,
    }
    v
}

fn jv3(v: Vec3) -> J {
    json!([v.x, v.y, v.z])
}

fn jq(q: Quat) -> J {
    json!([q.w, q.x, q.y, q.z])
}

fn jm4(m: &Mat4) -> J {
    json!(m.to_cols_array().to_vec())
}

fn reply(v: J) -> Res<Option<String>> {
    Ok(Some(format!("=json {v}")))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn f32s(v: impl IntoIterator<Item = f32>) -> String {
    b64(&v.into_iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
}

fn u32s(v: impl IntoIterator<Item = u32>) -> String {
    b64(&v.into_iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())
}

/// The mesh as JSON for the Python side's element views.
pub fn mesh_json(m: &Mesh, name: &str, sel: Option<&Sel>) -> J {
    let edges = m.edges();
    let mut j = json!({
        "name": name,
        "verts": f32s(m.positions.iter().flat_map(|p| p.to_array())),
        "loop_totals": u32s(m.faces.iter().map(|f| f.verts.len() as u32)),
        "loops": u32s(m.faces.iter().flat_map(|f| f.verts.iter().copied())),
        "edges": u32s(edges.iter().flat_map(|e| [e[0], e[1]])),
        "smooth": m.faces.iter().map(|f| if f.smooth { '1' } else { '0' }).collect::<String>(),
        "mat": m.faces.iter().map(|f| f.material).collect::<Vec<_>>(),
        "seams": m.seams.iter().map(|e| [e.0, e.1]).collect::<Vec<_>>(),
        "sharp": m.sharp_edges.iter().map(|e| [e.0, e.1]).collect::<Vec<_>>(),
        "vertex_groups": m.vertex_groups,
        "uvs": J::Null,
    });
    if m.has_uvs() {
        j["uvs"] = J::String(f32s(m.faces.iter().flat_map(|f| {
            let n = f.verts.len();
            (0..n).flat_map(move |c| f.uvs.get(c).map_or([0.0, 0.0], |u| [u.x, u.y]))
        })));
    }
    if !m.vertex_groups.is_empty() {
        let mut w = Vec::new();
        for (v, list) in m.weights.iter().enumerate() {
            for &(g, x) in list {
                w.extend([v as f32, g as f32, x]);
            }
        }
        j["weights"] = J::String(f32s(w));
    }
    if !m.shape_keys.is_empty() {
        let n = m.positions.len();
        let keys: Vec<J> = m
            .shape_keys
            .iter()
            .map(|k| {
                let mut pos = k.positions.clone();
                pos.truncate(n);
                pos.extend_from_slice(&m.positions[pos.len()..]);
                json!({
                    "name": k.name, "co": f32s(pos.iter().flat_map(|p| p.to_array())), "value": k.value,
                    "slider_min": k.slider_min, "slider_max": k.slider_max,
                    "relative_key": m.shape_keys.get(k.relative_key).map(|r| r.name.clone()).unwrap_or_default(),
                    "vertex_group": k.vertex_group.clone().unwrap_or_default(), "mute": k.mute,
                    "interpolation": format!("KEY_{}", k.interpolation.name().to_ascii_uppercase()), "frame": k.frame,
                })
            })
            .collect();
        j["shape_keys"] = J::Array(keys);
        j["use_relative"] = J::Bool(m.shape_key_settings.use_relative);
        j["eval_time"] = json!(m.shape_key_settings.eval_time);
    }
    if let Some(s) = sel {
        j["sel_v"] = json!(s.verts(m));
        j["sel_e"] = json!(s.edges(m).iter().map(|e| [e[0].min(e[1]), e[0].max(e[1])]).collect::<Vec<_>>());
        j["sel_f"] = json!(s.faces(m));
    }
    j
}

fn tree_json(scene: &Scene, t: &crate::nodes::NodeTree) -> J {
    let sock = |s: &crate::nodes::Socket, linked: bool| {
        json!({
            "identifier": s.identifier, "name": s.name, "type": nodes::socket_type_name(s.ty),
            "value": nodes::value_text(scene, &s.value, s.ty), "enabled": s.enabled, "hide": s.hide,
            "hide_value": s.hide_value, "is_linked": linked, "min": if s.min == f32::MIN { J::Null } else { json!(s.min) },
            "max": if s.max == f32::MAX { J::Null } else { json!(s.max) },
        })
    };
    let nodes: Vec<J> = t
        .nodes
        .iter()
        .enumerate()
        .map(|(ni, n)| {
            let ins: Vec<J> = n.inputs.iter().enumerate().map(|(si, s)| sock(s, t.links.iter().any(|l| l.to_node == ni && l.to_socket == si))).collect();
            let outs: Vec<J> = n.outputs.iter().enumerate().map(|(si, s)| sock(s, t.links.iter().any(|l| l.from_node == ni && l.from_socket == si))).collect();
            let props: serde_json::Map<String, J> = n.props.iter().map(|(k, p)| (k.clone(), J::String(nodes::prop_text(scene, p, k)))).collect();
            json!({
                "name": n.name, "bl_idname": n.idname, "label": n.label, "location": n.location, "width": n.width,
                "mute": n.mute, "hide": n.hide, "select": n.select, "parent": n.parent.and_then(|p| t.nodes.get(p)).map(|p| p.name.clone()),
                "color": n.color, "inputs": ins, "outputs": outs, "props": props,
            })
        })
        .collect();
    let links: Vec<J> = t
        .links
        .iter()
        .filter_map(|l| {
            let (f, to) = (t.nodes.get(l.from_node)?, t.nodes.get(l.to_node)?);
            Some(json!({
                "from_node": f.name, "from_socket": f.outputs.get(l.from_socket)?.identifier, "from_index": l.from_socket,
                "to_node": to.name, "to_socket": to.inputs.get(l.to_socket)?.identifier, "to_index": l.to_socket, "muted": l.muted,
            }))
        })
        .collect();
    let iface = |list: &[crate::nodes::InterfaceSocket]| -> Vec<J> {
        list.iter().map(|s| json!({"name": s.name, "type": nodes::socket_type_name(s.ty), "default": nodes::value_text(scene, &s.default, s.ty)})).collect()
    };
    json!({
        "name": t.name, "kind": format!("{:?}", t.kind), "nodes": nodes, "links": links,
        "inputs": iface(&t.inputs), "outputs": iface(&t.outputs), "active": t.active.and_then(|a| t.nodes.get(a)).map(|n| n.name.clone()),
    })
}

fn light_json(l: &Light) -> J {
    let (kind, extra) = match l.kind {
        LightKind::Point => ("POINT", json!({})),
        LightKind::Sun => ("SUN", json!({})),
        LightKind::Spot { angle, blend } => ("SPOT", json!({"spot_size": angle, "spot_blend": blend})),
        LightKind::Area { size } => ("AREA", json!({"size": size})),
    };
    let mut j = json!({
        "type": kind, "energy": l.power, "color": jv3(l.color), "shadow_soft_size": l.radius, "angle": l.radius,
        "shape": shape_name(l.shape).to_ascii_uppercase(), "size_y": l.size_y, "spread": l.spread, "max_bounces": l.max_bounces,
        "cast_shadow": l.cast_shadow, "use_mis": l.use_mis,
    });
    if let (Some(o), J::Object(e)) = (j.as_object_mut(), extra) {
        o.extend(e);
    }
    j
}

fn camera_json(c: &Camera) -> J {
    let mut j = json!({
        "type": if c.panorama.is_some() { "PANO" } else if c.ortho_scale.is_some() { "ORTHO" } else { "PERSP" },
        "angle": c.fov_y, "lens": fov_to_lens(c.fov_y), "clip_start": c.near, "clip_end": c.far,
        "ortho_scale": c.ortho_scale.unwrap_or(6.0), "sensor_width": c.sensor_width, "sensor_height": c.sensor_height,
        "shift_x": c.shift_x, "shift_y": c.shift_y, "sensor_fit": fit_name(c.sensor_fit).to_ascii_uppercase(),
    });
    for key in CAMERA_OPTS {
        if let Some(v) = camera_opt_value(c, key) {
            j[key.to_string()] = J::String(v);
        }
    }
    j
}

fn object_type(o: &Object) -> &'static str {
    match o.data {
        ObjectData::Empty => "EMPTY",
        ObjectData::Mesh(_) => "MESH",
        ObjectData::Camera(_) => "CAMERA",
        ObjectData::Light(_) => "LIGHT",
        ObjectData::Armature(_) => "ARMATURE",
        ObjectData::Lattice(_) => "LATTICE",
    }
}

fn data_name(scene: &Scene, o: &Object) -> J {
    match o.data {
        ObjectData::Empty => J::Null,
        ObjectData::Mesh(m) => json!(scene.mesh_names[m]),
        ObjectData::Armature(a) => json!(scene.armature_names[a]),
        ObjectData::Lattice(l) => json!(scene.lattice_names[l]),
        ObjectData::Camera(_) | ObjectData::Light(_) => json!(o.name),
    }
}

/// Parse a 16-number column-major matrix option.
fn mat_of(v: &[f32]) -> Res<Mat4> {
    let a: [f32; 16] = v.try_into().map_err(|_| format!("a matrix needs 16 numbers, got {}", v.len()))?;
    Ok(Mat4::from_cols_array(a))
}

/// Selection of the edit mesh, or every element when nothing is stored.
fn mesh_sel<'a>(st: &'a mut State, mi: usize) -> &'a mut Sel {
    st.sel.entry(mi).or_default()
}

/// Every operator idname handled by [`Session::run_op`].
pub const OPERATORS: &[&str] = &[
    "mesh.primitive_cube_add", "mesh.primitive_plane_add", "mesh.primitive_grid_add", "mesh.primitive_uv_sphere_add", "mesh.primitive_ico_sphere_add",
    "mesh.primitive_cylinder_add", "mesh.primitive_cone_add", "mesh.primitive_torus_add", "mesh.primitive_monkey_add", "mesh.primitive_circle_add",
    "mesh.select_all", "mesh.select_mode", "mesh.select_non_manifold", "mesh.select_linked", "mesh.select_more", "mesh.select_less",
    "mesh.extrude_region_move", "mesh.extrude_region", "mesh.extrude_faces_move", "mesh.extrude_region_shrink_fatten", "mesh.extrude_repeat",
    "mesh.subdivide", "mesh.delete", "mesh.remove_doubles", "mesh.merge", "mesh.normals_make_consistent", "mesh.flip_normals", "mesh.inset", "mesh.bevel",
    "mesh.quads_convert_to_tris", "mesh.tris_convert_to_quads", "mesh.fill", "mesh.edge_face_add", "mesh.poke", "mesh.dissolve_verts", "mesh.dissolve_edges",
    "mesh.dissolve_faces", "mesh.dissolve_limited", "mesh.spin", "mesh.bisect", "mesh.convex_hull", "mesh.symmetrize", "mesh.mark_seam", "mesh.mark_sharp",
    "mesh.faces_shade_smooth", "mesh.faces_shade_flat", "mesh.loopcut", "mesh.loopcut_slide", "mesh.duplicate_move", "mesh.duplicate", "mesh.unsubdivide",
    "mesh.edge_split", "mesh.split", "mesh.fill_grid", "mesh.beautify_fill", "mesh.reveal", "mesh.hide",
    "object.select_all", "object.select_by_type", "object.delete", "object.duplicate", "object.duplicate_move", "object.duplicate_move_linked", "object.join",
    "object.shade_smooth", "object.shade_flat", "object.shade_auto_smooth", "object.modifier_add", "object.modifier_remove", "object.modifier_apply",
    "object.modifier_move_up", "object.modifier_move_down", "object.modifier_move_to_index", "object.transform_apply", "object.origin_set", "object.parent_set",
    "object.parent_clear", "object.convert", "object.location_clear", "object.rotation_clear", "object.scale_clear", "object.hide_view_set",
    "object.hide_view_clear", "object.material_slot_add", "object.material_slot_remove", "object.vertex_group_add", "object.vertex_group_remove",
    "object.shape_key_add", "object.mode_set", "object.editmode_toggle", "object.posemode_toggle", "object.light_add", "object.camera_add", "object.empty_add",
    "object.armature_add", "object.constraint_add", "object.make_single_user",
    "transform.translate", "transform.rotate", "transform.resize", "transform.mirror",
    "uv.smart_project", "uv.unwrap", "uv.cube_project", "uv.cylinder_project", "uv.sphere_project", "uv.reset", "uv.lightmap_pack", "uv.project_from_view",
    "pose.transforms_clear", "pose.rot_clear", "pose.loc_clear", "pose.scale_clear", "pose.select_all",
    "armature.bone_primitive_add", "armature.select_all",
    "anim.keyframe_insert", "anim.keyframe_insert_by_name", "anim.keyframe_delete",
    "screen.frame_jump", "screen.frame_offset",
    "render.render",
    "wm.save_mainfile", "wm.save_as_mainfile", "wm.open_mainfile", "wm.read_homefile", "wm.read_factory_settings",
    "import_scene.gltf", "import_scene.obj", "wm.obj_import", "wm.stl_import", "import_mesh.stl", "wm.ply_import", "import_mesh.ply",
    "export_scene.gltf", "export_scene.obj", "wm.obj_export", "wm.stl_export", "export_mesh.stl", "wm.ply_export", "export_mesh.ply",
];

impl Session {
    fn edit_mesh(&self, scene: &Scene) -> Res<usize> {
        let o = scene.active_object.ok_or("no active object")?;
        match scene.objects[o].data {
            ObjectData::Mesh(m) if self.bpy.mode == "EDIT" => Ok(m),
            _ => Err("this operator needs a mesh in Edit Mode".into()),
        }
    }

    /// Objects an object-mode operator acts on: the selection, else the active.
    fn targets(scene: &Scene) -> Vec<usize> {
        if scene.selected.is_empty() {
            scene.active_object.into_iter().collect()
        } else {
            scene.selected.clone()
        }
    }

    fn op_poll(&self, scene: &Scene, id: &str) -> bool {
        if !OPERATORS.contains(&id) {
            return false;
        }
        let edit_mesh = self.edit_mesh(scene).is_ok();
        let active = scene.active_object.is_some();
        let (cat, name) = id.split_once('.').unwrap_or(("", id));
        match cat {
            "mesh" if name.starts_with("primitive_") => true,
            "mesh" => edit_mesh,
            "uv" => edit_mesh || active,
            "pose" => self.bpy.mode == "POSE",
            "armature" => self.bpy.mode == "EDIT" && scene.active_object.is_some_and(|o| matches!(scene.objects[o].data, ObjectData::Armature(_))),
            "object" => match name {
                "modifier_add" | "modifier_remove" | "modifier_apply" | "modifier_move_up" | "modifier_move_down" | "modifier_move_to_index" | "material_slot_add"
                | "material_slot_remove" | "vertex_group_add" | "vertex_group_remove" | "shape_key_add" | "editmode_toggle" | "posemode_toggle" | "constraint_add" => active,
                "join" => active && scene.selected.len() > 1,
                "delete" | "duplicate" | "duplicate_move" | "duplicate_move_linked" | "transform_apply" | "origin_set" | "parent_set" | "convert" => !Self::targets(scene).is_empty() && self.bpy.mode == "OBJECT",
                _ => true,
            },
            "transform" => active || !scene.selected.is_empty(),
            _ => true,
        }
    }

    pub(super) fn bpy_command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        match cmd.name.as_str() {
            "ids" => {
                cmd.max_args(0)?;
                let names = |v: Vec<&str>| json!(v);
                let objs = |f: fn(&Object) -> bool| names(scene.objects.iter().filter(|o| f(o)).map(|o| o.name.as_str()).collect());
                reply(json!({
                    "objects": objs(|_| true),
                    "meshes": scene.mesh_names,
                    "materials": names(scene.materials.iter().map(|m| m.name.as_str()).collect()),
                    "collections": names(scene.collections.iter().map(|c| c.name.as_str()).collect()),
                    "images": names(scene.images.iter().map(|i| i.name.as_str()).collect()),
                    "node_groups": names(scene.node_groups.iter().map(|g| g.name.as_str()).collect()),
                    "armatures": scene.armature_names,
                    "lattices": scene.lattice_names,
                    "lights": objs(|o| matches!(o.data, ObjectData::Light(_))),
                    "cameras": objs(|o| matches!(o.data, ObjectData::Camera(_))),
                    "actions": names(self.bpy.actions.iter().map(|a| a.name.as_str()).collect()),
                    "scene": scene.name,
                    "selected": names(scene.selected.iter().filter_map(|&i| scene.objects.get(i)).map(|o| o.name.as_str()).collect()),
                    "active": scene.active_object.and_then(|i| scene.objects.get(i)).map(|o| o.name.clone()),
                    "camera": scene.active_camera.and_then(|i| scene.objects.get(i)).map(|o| o.name.clone()),
                    "mode": self.bpy.mode,
                    "frame": scene.frame_current,
                }))
            }
            "dump_object" => {
                cmd.max_args(1)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let o = &scene.objects[i];
                let order = self.bpy.euler_order(&o.name);
                let euler = self.bpy.eulers.get(&o.name).copied().filter(|e| Quat::from_euler(*e, order).dot(o.transform.rotation).abs() > 0.99999).unwrap_or_else(|| o.transform.rotation.to_euler(order));
                let world = scene.world_matrix(i);
                let parent_world = o.parent.map(|p| scene.world_matrix(p)).unwrap_or(Mat4::IDENTITY);
                let mut rig = String::new();
                rig::write_object_rig(&mut rig, o);
                let mods: Vec<J> = o.modifiers.iter().map(|m| json!({"type": m.type_name(), "name": m.name(), "opts": modifier_to_options(m)})).collect();
                let bounds = match o.data {
                    ObjectData::Mesh(m) => {
                        let b = scene.meshes[m].bounds();
                        if b.is_empty() {
                            J::Null
                        } else {
                            json!([b.min.to_array(), b.max.to_array()])
                        }
                    }
                    _ => J::Null,
                };
                let mut j = json!({
                    "name": o.name, "type": object_type(o), "data": data_name(scene, o),
                    "location": jv3(o.transform.location), "rotation_quaternion": jq(o.transform.rotation), "rotation_euler": euler,
                    "rotation_mode": self.bpy.rotation_modes.get(&o.name).cloned().unwrap_or_else(|| "XYZ".into()),
                    "scale": jv3(o.transform.scale), "matrix_world": jm4(&world), "matrix_parent_world": jm4(&parent_world),
                    "parent": o.parent.map(|p| scene.objects[p].name.clone()), "parent_type": format!("{:?}", o.parent_type).to_ascii_uppercase(),
                    "parent_bone": o.parent_bone, "collection": scene.collections.get(o.collection).map(|c| c.name.clone()),
                    "hide_viewport": !o.visible, "hide_render": self.bpy.hide_render.contains(&o.name),
                    "select": scene.selected.contains(&i), "active": scene.active_object == Some(i),
                    "materials": o.materials.iter().map(|&m| scene.materials.get(m).map(|x| x.name.clone())).collect::<Vec<_>>(),
                    "modifiers": mods, "rig": rig, "pass_index": o.pass_index, "holdout": o.holdout,
                    "visible_camera": o.visibility.camera, "visible_diffuse": o.visibility.diffuse, "visible_glossy": o.visibility.glossy,
                    "visible_transmission": o.visibility.transmission, "visible_volume_scatter": o.visibility.volume_scatter, "visible_shadow": o.visibility.shadow,
                    "action": self.bpy.anim.get(&o.name), "bounds": bounds,
                    "children": scene.objects.iter().filter(|c| c.parent == Some(i)).map(|c| c.name.clone()).collect::<Vec<_>>(),
                });
                match &o.data {
                    ObjectData::Light(l) => j["light"] = light_json(l),
                    ObjectData::Camera(c) => j["camera"] = camera_json(c),
                    ObjectData::Armature(_) => {
                        let mats = scene.pose_matrices(i);
                        if let Some(arm) = scene.armature_of(i) {
                            let pm: serde_json::Map<String, J> = arm.bones.iter().zip(mats.iter()).map(|(b, m)| (b.name.clone(), jm4(m))).collect();
                            j["pose_matrices"] = J::Object(pm);
                            j["bone_select"] = json!(self.bpy.bone_sel.get(&o.name).cloned().unwrap_or_default());
                        }
                    }
                    _ => {}
                }
                reply(j)
            }
            "dump_mesh" | "eval_mesh" => {
                cmd.max_args(1)?;
                if cmd.name == "eval_mesh" {
                    let i = self.object(scene, cmd.arg(0, "an object")?)?;
                    let m = scene.evaluated_mesh(i).ok_or("the object has no mesh")?;
                    return reply(mesh_json(&m, &scene.objects[i].name, None));
                }
                let mi = self.mesh(scene, cmd.arg(0, "a mesh")?)?;
                let users: Vec<&str> = scene.objects.iter().filter(|o| matches!(o.data, ObjectData::Mesh(x) if x == mi)).map(|o| o.name.as_str()).collect();
                let mut j = mesh_json(&scene.meshes[mi], &scene.mesh_names[mi], Some(self.bpy.sel.get(&mi).unwrap_or(&Sel::default())));
                j["users"] = json!(users);
                j["is_editmode"] = json!(self.bpy.mode == "EDIT" && scene.active_object.is_some_and(|a| matches!(scene.objects[a].data, ObjectData::Mesh(x) if x == mi)));
                reply(j)
            }
            "dump_material" => {
                cmd.max_args(1)?;
                let mi = self.material(scene, cmd.arg(0, "a material")?)?;
                let m = &scene.materials[mi];
                reply(json!({
                    "name": m.name, "base_color": m.base_color, "metallic": m.metallic, "roughness": m.roughness, "emission": jv3(m.emission),
                    "emission_strength": m.emission_strength, "ior": m.ior, "transmission": m.transmission, "double_sided": m.double_sided,
                    "pass_index": m.pass_index, "use_nodes": m.node_tree.is_some(),
                    "users": scene.objects.iter().filter(|o| o.materials.contains(&mi)).count(),
                }))
            }
            "dump_tree" => {
                let owner = cmd.arg(0, "material NAME, world or group NAME")?.text.clone();
                let t = match owner.as_str() {
                    "material" => {
                        cmd.max_args(2)?;
                        let mi = self.material(scene, cmd.arg(1, "a material")?)?;
                        scene.materials[mi].node_tree.as_ref()
                    }
                    "world" => {
                        cmd.max_args(1)?;
                        scene.world.node_tree.as_ref()
                    }
                    "group" => {
                        cmd.max_args(2)?;
                        let n = &cmd.arg(1, "a group")?.text;
                        Some(scene.node_groups.iter().find(|g| g.name == *n).ok_or_else(|| format!("no node group named '{n}'"))?)
                    }
                    "compositor" => scene.compositor.as_ref(),
                    o => return Err(format!("unknown tree owner '{o}'")),
                };
                reply(t.map(|t| tree_json(scene, t)).unwrap_or(J::Null))
            }
            "dump_scene" => {
                cmd.max_args(0)?;
                let w = &scene.world;
                let colls: Vec<J> = scene
                    .collections
                    .iter()
                    .enumerate()
                    .map(|(ci, c)| {
                        json!({
                            "name": c.name, "parent": c.parent.and_then(|p| scene.collections.get(p)).map(|p| p.name.clone()), "hide_viewport": !c.visible,
                            "objects": scene.objects.iter().filter(|o| o.collection == ci).map(|o| o.name.clone()).collect::<Vec<_>>(),
                            "children": scene.collections.iter().filter(|x| x.parent == Some(ci)).map(|x| x.name.clone()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                reply(json!({
                    "name": scene.name, "frame_current": scene.frame_current, "frame_start": scene.frame_start, "frame_end": scene.frame_end, "fps": scene.fps,
                    "camera": scene.active_camera.and_then(|i| scene.objects.get(i)).map(|o| o.name.clone()),
                    "world": {"color": jv3(w.color), "strength": w.strength, "use_nodes": w.node_tree.is_some()},
                    "collections": colls, "props": self.bpy.props, "cursor": self.bpy.cursor, "mode": self.bpy.mode,
                    "select_mode": self.bpy.select_mode, "use_nodes": scene.compositor.is_some(),
                }))
            }
            "dump_armature" => {
                cmd.max_args(1)?;
                let t = cmd.arg(0, "an armature")?.clone();
                let ai = self.armature_data_pub(scene, &t)?;
                let arm = &scene.armatures[ai];
                let rest = arm.rest_matrices();
                let bones: Vec<J> = arm
                    .bones
                    .iter()
                    .enumerate()
                    .map(|(bi, b)| {
                        json!({
                            "name": b.name, "parent": arm.parent(bi).map(|p| arm.bones[p].name.clone()), "head": jv3(b.head), "tail": jv3(b.tail), "roll": b.roll,
                            "use_connect": b.use_connect, "use_deform": b.use_deform, "use_inherit_rotation": b.use_inherit_rotation,
                            "inherit_scale": format!("{:?}", b.inherit_scale).to_ascii_uppercase(), "use_local_location": b.use_local_location,
                            "use_relative_parent": b.use_relative_parent, "envelope_distance": b.envelope_distance, "envelope_weight": b.envelope_weight,
                            "head_radius": b.head_radius, "tail_radius": b.tail_radius, "bbone_segments": b.bbone_segments, "collections": b.collections,
                            "matrix_local": rest.get(bi).map(jm4), "length": b.length(),
                            "children": arm.bones.iter().enumerate().filter(|(ci, _)| arm.parent(*ci) == Some(bi)).map(|(_, c)| c.name.clone()).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                reply(json!({"name": scene.armature_names[ai], "bones": bones, "collections": arm.collections}))
            }
            "dump_lattice" => {
                cmd.max_args(1)?;
                let n = cmd.arg(0, "a lattice")?.text.clone();
                let li = scene.lattice_names.iter().position(|x| *x == n).ok_or_else(|| format!("no lattice named '{n}'"))?;
                let l = &scene.lattices[li];
                let interp: Vec<String> = [l.interpolation_u, l.interpolation_v, l.interpolation_w].iter().map(|i| format!("KEY_{}", i.name().to_ascii_uppercase())).collect();
                reply(json!({
                    "name": n, "points_u": l.points_u, "points_v": l.points_v, "points_w": l.points_w, "use_outside": l.use_outside,
                    "points": f32s(l.points.iter().flat_map(|p| p.to_array())), "rest": f32s(l.rest_points().iter().flat_map(|p| p.to_array())),
                    "interpolation": interp,
                }))
            }
            "dump_image" => {
                cmd.max_args(1)?;
                let n = cmd.arg(0, "an image")?.text.clone();
                let pixels = cmd.b("pixels")?.unwrap_or(false);
                let img = scene.images.iter().find(|x| x.name == n).ok_or_else(|| format!("no image named '{n}'"))?;
                let mut j = json!({"name": img.name, "size": [img.width, img.height]});
                if pixels {
                    // Blender's order: bottom row first.
                    let w = img.width as usize;
                    let rows: Vec<f32> = (0..img.height as usize).rev().flat_map(|y| img.pixels[y * w..(y + 1) * w].iter().flat_map(|p| *p)).collect();
                    j["pixels"] = J::String(f32s(rows));
                }
                reply(j)
            }
            "dump_action" => {
                cmd.max_args(1)?;
                let n = cmd.arg(0, "an action")?.text.clone();
                let a = self.bpy.action(&n).ok_or_else(|| format!("no action named '{n}'"))?;
                let curves: Vec<J> = a
                    .fcurves
                    .iter()
                    .map(|c| {
                        json!({
                            "data_path": c.data_path, "array_index": c.index, "group": c.group, "mute": c.mute,
                            "keys": c.keys.iter().map(|k| json!({"co": k.co, "interpolation": k.interpolation, "handle_left": k.handle_left, "handle_right": k.handle_right, "handle_type": k.handle_type, "select": k.select})).collect::<Vec<_>>(),
                        })
                    })
                    .collect();
                reply(json!({"name": a.name, "fcurves": curves, "frame_range": a.frame_range(), "users": self.bpy.anim.values().filter(|x| **x == n).count()}))
            }
            "op_poll" => {
                cmd.max_args(1)?;
                let id = cmd.arg(0, "an operator idname")?.text.clone();
                Ok(Some(if self.op_poll(scene, &id) { "1" } else { "0" }.into()))
            }
            "op_list" => {
                cmd.max_args(0)?;
                reply(json!(OPERATORS))
            }
            "op" => {
                let id = cmd.arg(0, "an operator idname")?.text.clone();
                if !OPERATORS.contains(&id.as_str()) {
                    return Err(format!("unknown operator '{id}'"));
                }
                if !self.op_poll(scene, &id) {
                    for o in cmd.opts.iter_mut() {
                        o.2 = true;
                    }
                    return Err(format!("Operator bpy.ops.{id}.poll() failed, context is incorrect"));
                }
                let r = self.run_op(scene, cmd, &id);
                // Operators accept and ignore arguments they have no use for.
                for o in cmd.opts.iter_mut() {
                    o.2 = true;
                }
                r.map(|ok| Some(if ok { "FINISHED" } else { "CANCELLED" }.into()))
            }
            "obj_set" => {
                cmd.max_args(1)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let name = scene.objects[i].name.clone();
                if let Some(t) = cmd.opt("rotation_mode") {
                    let m = t.text.to_ascii_uppercase();
                    if !matches!(m.as_str(), "QUATERNION" | "AXIS_ANGLE" | "XYZ" | "XZY" | "YXZ" | "YZX" | "ZXY" | "ZYX") {
                        return Err(format!("rotation_mode '{}' is not QUATERNION, AXIS_ANGLE or an Euler order", t.text));
                    }
                    self.bpy.rotation_modes.insert(name.clone(), m);
                    self.bpy.eulers.remove(&name);
                }
                if let Some(e) = cmd.v3("rotation_euler")? {
                    let order = self.bpy.euler_order(&name);
                    scene.objects[i].transform.rotation = Quat::from_euler(e.to_array(), order);
                    self.bpy.eulers.insert(name.clone(), e.to_array());
                }
                if let Some(q) = cmd.list("rotation_quaternion")? {
                    scene.objects[i].transform.rotation = quat_of(&q)?;
                    self.bpy.eulers.remove(&name);
                }
                if let Some(v) = cmd.v3("location")? {
                    scene.objects[i].transform.location = v;
                }
                if let Some(s) = cmd.list("scale")? {
                    scene.objects[i].transform.scale = scale_of(&s)?;
                }
                if let Some(v) = cmd.list("matrix_world")? {
                    let m = mat_of(&v)?;
                    let pw = scene.objects[i].parent.map(|p| scene.world_matrix(p)).unwrap_or(Mat4::IDENTITY);
                    scene.objects[i].transform = Transform::from_matrix(&(pw.inverse() * m));
                    self.bpy.eulers.remove(&name);
                }
                if let Some(v) = cmd.list("matrix_local")? {
                    scene.objects[i].transform = Transform::from_matrix(&mat_of(&v)?);
                    self.bpy.eulers.remove(&name);
                }
                if let Some(h) = cmd.b("hide_render")? {
                    if h {
                        self.bpy.hide_render.insert(name.clone());
                    } else {
                        self.bpy.hide_render.remove(&name);
                    }
                }
                if let Some(sel) = cmd.b("select")? {
                    scene.selected.retain(|&x| x != i);
                    if sel {
                        scene.selected.push(i);
                    }
                }
                Ok(None)
            }
            "mesh_select" => {
                cmd.max_args(1)?;
                let mi = self.mesh(scene, cmd.arg(0, "a mesh")?)?;
                let m = &scene.meshes[mi];
                let (nv, nf) = (m.positions.len() as u32, m.faces.len() as u32);
                let s = mesh_sel(&mut self.bpy, mi);
                if let Some(t) = cmd.opt("verts") {
                    s.verts = uints(&t.text)?.into_iter().filter(|&v| v < nv).collect();
                }
                if let Some(t) = cmd.opt("edges") {
                    let e = uints(&t.text)?;
                    s.edges = e.chunks_exact(2).filter(|c| c[0] < nv && c[1] < nv).map(|c| (c[0].min(c[1]), c[0].max(c[1]))).collect();
                }
                if let Some(t) = cmd.opt("faces") {
                    s.faces = uints(&t.text)?.into_iter().filter(|&f| f < nf).collect();
                }
                if let Some(t) = cmd.opt("mode") {
                    let w = t.text.to_ascii_uppercase();
                    self.bpy.select_mode = [w.contains("VERT"), w.contains("EDGE"), w.contains("FACE")];
                }
                Ok(None)
            }
            "mesh_attr" => {
                cmd.max_args(1)?;
                let mi = self.mesh(scene, cmd.arg(0, "a mesh")?)?;
                let m = &mut scene.meshes[mi];
                let nf = m.faces.len();
                if let Some(t) = cmd.opt("smooth") {
                    let s = t.text.trim();
                    if s.len() != nf || !s.bytes().all(|b| b == b'0' || b == b'1') {
                        return Err(format!("smooth= is one 0/1 per face ({nf} faces)"));
                    }
                    for (f, b) in m.faces.iter_mut().zip(s.bytes()) {
                        f.smooth = b == b'1';
                    }
                }
                if let Some(t) = cmd.opt("mat") {
                    let v = uints(&t.text)?;
                    if v.len() != nf {
                        return Err(format!("mat= has {} entries for {nf} faces", v.len()));
                    }
                    for (f, s) in m.faces.iter_mut().zip(v) {
                        f.material = s.min(255) as u16;
                    }
                }
                if let Some(uv) = cmd.list("uvs")? {
                    let corners: usize = m.faces.iter().map(|f| f.verts.len()).sum();
                    if uv.len() != corners * 2 {
                        return Err(format!("uvs= holds {} numbers for {corners} face corners (2 each)", uv.len()));
                    }
                    let mut k = 0;
                    for f in &mut m.faces {
                        f.uvs = (0..f.verts.len()).map(|c| Vec2::new(uv[k + 2 * c], uv[k + 2 * c + 1])).collect();
                        k += 2 * f.verts.len();
                    }
                }
                if let Some(t) = cmd.opt("clear_uvs") {
                    if boolean(&t.text)? {
                        m.faces.iter_mut().for_each(|f| f.uvs.clear());
                    }
                }
                if let Some(v) = cmd.list("verts")? {
                    if v.len() != m.positions.len() * 3 {
                        return Err(format!("verts= holds {} numbers for {} vertices", v.len(), m.positions.len()));
                    }
                    m.positions = v.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
                }
                if let Some(t) = cmd.opt("seams") {
                    m.seams = edge_pairs(&t.text, "seams")?.into_iter().map(|e| crate::mesh::edge_key(e[0], e[1])).collect();
                }
                if let Some(t) = cmd.opt("sharp") {
                    m.sharp_edges = edge_pairs(&t.text, "sharp")?.into_iter().map(|e| crate::mesh::edge_key(e[0], e[1])).collect();
                }
                Ok(None)
            }
            "mode_set" => {
                cmd.max_args(1)?;
                let mode = cmd.arg(0, "a mode")?.text.to_ascii_uppercase();
                self.set_mode(scene, &mode)?;
                Ok(None)
            }
            "prop_set" => {
                cmd.max_args(2)?;
                let k = cmd.arg(0, "a property path")?.text.clone();
                let v = cmd.arg(1, "a value")?.text.clone();
                match k.as_str() {
                    "scene.frame_current" => {
                        scene.frame_current = v.trim().parse::<f32>().map_err(|_| format!("'{v}' is not a frame"))? as i32;
                    }
                    "scene.frame_start" => scene.frame_start = num(&v)? as i32,
                    "scene.frame_end" => scene.frame_end = num(&v)? as i32,
                    "scene.cursor.location" => self.bpy.cursor = vec3_of(&floats(&v)?, "cursor")?.to_array(),
                    "render.fps" => {
                        scene.fps = num(&v)?;
                        self.bpy.props.insert(k, v);
                    }
                    _ => {
                        self.bpy.props.insert(k, v);
                    }
                }
                Ok(None)
            }
            "frame_set" => {
                cmd.max_args(2)?;
                let f = num(&cmd.arg(0, "a frame")?.text)?;
                let sub = match cmd.pos.get(1) {
                    Some(t) => num(&t.text)?,
                    None => 0.0,
                };
                scene.frame_current = f as i32;
                self.bpy.evaluate(scene, f + sub);
                Ok(None)
            }
            "keyframe" => {
                // keyframe OBJ PATH INDEX FRAME VALUE [interpolation=] [group=]
                cmd.max_args(5)?;
                let oi = self.object(scene, cmd.arg(0, "an object")?)?;
                let obj = scene.objects[oi].name.clone();
                let path = cmd.arg(1, "a data path")?.text.clone();
                let index = uint(&cmd.arg(2, "an index")?.text)?;
                let frame = num(&cmd.arg(3, "a frame")?.text)?;
                let value = num(&cmd.arg(4, "a value")?.text)?;
                let interp = cmd.opt("interpolation").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "BEZIER".into());
                let group = cmd.opt("group").map(|t| t.text).unwrap_or_else(|| default_group(&path));
                let act = match self.bpy.anim.get(&obj) {
                    Some(a) if self.bpy.action(a).is_some() => a.clone(),
                    _ => {
                        let n = unique_name(&format!("{obj}Action"), |n| self.bpy.actions.iter().any(|a| a.name == n));
                        self.bpy.actions.push(Action { name: n.clone(), fcurves: Vec::new() });
                        self.bpy.anim.insert(obj.clone(), n.clone());
                        n
                    }
                };
                let a = self.bpy.action_mut(&act).expect("action exists");
                a.fcurve_mut(&path, index, &group).insert(frame, value, &interp);
                Ok(Some(quote(&act)))
            }
            "keyframe_delete" => {
                cmd.max_args(4)?;
                let oi = self.object(scene, cmd.arg(0, "an object")?)?;
                let obj = scene.objects[oi].name.clone();
                let path = cmd.arg(1, "a data path")?.text.clone();
                let index = cmd.arg(2, "an index")?.text.parse::<i64>().map_err(|_| "index must be a whole number")?;
                let frame = num(&cmd.arg(3, "a frame")?.text)?;
                let Some(act) = self.bpy.anim.get(&obj).cloned() else { return Ok(Some("0".into())) };
                let mut n = 0;
                if let Some(a) = self.bpy.action_mut(&act) {
                    for c in a.fcurves.iter_mut().filter(|c| c.data_path == path && (index < 0 || c.index as i64 == index)) {
                        let before = c.keys.len();
                        c.keys.retain(|k| (k.co[0] - frame).abs() >= 0.01);
                        n += before - c.keys.len();
                        c.recalc();
                    }
                    a.fcurves.retain(|c| !c.keys.is_empty());
                }
                Ok(Some(n.to_string()))
            }
            "action_new" => {
                cmd.max_args(1)?;
                let base = cmd.arg(0, "a name")?.text.clone();
                let n = unique_name(&base, |n| self.bpy.actions.iter().any(|a| a.name == n));
                self.bpy.actions.push(Action { name: n.clone(), fcurves: Vec::new() });
                Ok(Some(quote(&n)))
            }
            "action_assign" => {
                cmd.max_args(2)?;
                let oi = self.object(scene, cmd.arg(0, "an object")?)?;
                let obj = scene.objects[oi].name.clone();
                let t = cmd.arg(1, "an action or none")?.clone();
                if t.is_word("none") {
                    self.bpy.anim.remove(&obj);
                } else {
                    self.bpy.action(&t.text).ok_or_else(|| format!("no action named '{}'", t.text))?;
                    self.bpy.anim.insert(obj, t.text);
                }
                Ok(None)
            }
            "fcurve_new" | "fcurve_remove" => {
                cmd.max_args(3)?;
                let an = cmd.arg(0, "an action")?.text.clone();
                let path = cmd.arg(1, "a data path")?.text.clone();
                let index = uint(&cmd.arg(2, "an index")?.text)?;
                let group = cmd.opt("group").map(|t| t.text).unwrap_or_default();
                let a = self.bpy.action_mut(&an).ok_or_else(|| format!("no action named '{an}'"))?;
                if cmd.name == "fcurve_new" {
                    if a.fcurves.iter().any(|c| c.data_path == path && c.index == index) {
                        return Err(format!("F-Curve '{path}[{index}]' already exists in action '{an}'"));
                    }
                    a.fcurve_mut(&path, index, &group);
                } else {
                    a.fcurves.retain(|c| !(c.data_path == path && c.index == index));
                }
                Ok(None)
            }
            "key_insert" | "key_set" | "key_remove" => {
                // key_insert ACT PATH INDEX FRAME VALUE | key_set ACT PATH INDEX K [co=] [interpolation=] [handle_left=] [handle_right=] [handle_type=] [select=]
                let an = cmd.arg(0, "an action")?.text.clone();
                let path = cmd.arg(1, "a data path")?.text.clone();
                let index = uint(&cmd.arg(2, "an index")?.text)?;
                let name = cmd.name.clone();
                let a = self.bpy.action_mut(&an).ok_or_else(|| format!("no action named '{an}'"))?;
                let c = a.fcurves.iter_mut().find(|c| c.data_path == path && c.index == index).ok_or_else(|| format!("no F-Curve '{path}[{index}]'"))?;
                match name.as_str() {
                    "key_insert" => {
                        cmd.max_args(5)?;
                        let f = num(&cmd.arg(3, "a frame")?.text)?;
                        let v = num(&cmd.arg(4, "a value")?.text)?;
                        let interp = cmd.opt("interpolation").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "BEZIER".into());
                        Ok(Some(c.insert(f, v, &interp).to_string()))
                    }
                    "key_remove" => {
                        cmd.max_args(4)?;
                        let k = uint(&cmd.arg(3, "a key index")?.text)? as usize;
                        if k >= c.keys.len() {
                            return Err(format!("keyframe {k} does not exist"));
                        }
                        c.keys.remove(k);
                        c.recalc();
                        Ok(None)
                    }
                    _ => {
                        cmd.max_args(4)?;
                        let k = uint(&cmd.arg(3, "a key index")?.text)? as usize;
                        if k >= c.keys.len() {
                            return Err(format!("keyframe {k} does not exist"));
                        }
                        let key = &mut c.keys[k];
                        if let Some(v) = cmd.list("co")? {
                            key.co = [*v.first().ok_or("co= needs 2 numbers")?, *v.get(1).ok_or("co= needs 2 numbers")?];
                        }
                        if let Some(t) = cmd.opt("interpolation") {
                            key.interpolation = t.text.to_ascii_uppercase();
                        }
                        if let Some(t) = cmd.opt("handle_type") {
                            key.handle_type = t.text.to_ascii_uppercase();
                        }
                        if let Some(v) = cmd.list("handle_left")? {
                            key.handle_left = [v[0], *v.get(1).unwrap_or(&key.co[1])];
                            key.handle_type = "FREE".into();
                        }
                        if let Some(v) = cmd.list("handle_right")? {
                            key.handle_right = [v[0], *v.get(1).unwrap_or(&key.co[1])];
                            key.handle_type = "FREE".into();
                        }
                        if let Some(s) = cmd.b("select")? {
                            key.select = s;
                        }
                        c.recalc();
                        Ok(None)
                    }
                }
            }
            "register" | "unregister" => {
                cmd.max_args(2)?;
                let kind = cmd.arg(0, "a kind")?.text.clone();
                let idname = cmd.arg(1, "an idname")?.text.clone();
                self.bpy.registry.retain(|r| !(r.kind == kind && r.idname == idname));
                if cmd.name == "register" {
                    let attrs = cmd.opts.iter_mut().map(|o| {
                        o.2 = true;
                        (o.0.clone(), o.1.text.clone())
                    });
                    let attrs = attrs.collect();
                    self.bpy.registry.push(Registered { kind, idname, attrs });
                }
                Ok(None)
            }
            "report" => {
                cmd.max_args(2)?;
                let ty = cmd.arg(0, "a type")?.text.to_ascii_uppercase();
                let msg = cmd.arg(1, "a message")?.text.clone();
                self.bpy.reports.push((ty, msg));
                Ok(None)
            }
            "image_new" => {
                cmd.max_args(3)?;
                let base = cmd.arg(0, "a name")?.text.clone();
                let w = uint(&cmd.arg(1, "a width")?.text)?.clamp(1, 16384);
                let h = uint(&cmd.arg(2, "a height")?.text)?.clamp(1, 16384);
                let name = unique_name(&base, |n| scene.images.iter().any(|i| i.name == n));
                let mut img = crate::scene::Image::new(&name, w, h);
                if let Some(c) = cmd.list("color")? {
                    let c = [c.first().copied().unwrap_or(0.0), c.get(1).copied().unwrap_or(0.0), c.get(2).copied().unwrap_or(0.0), c.get(3).copied().unwrap_or(1.0)];
                    img.pixels.iter_mut().for_each(|p| *p = c);
                }
                scene.images.push(img);
                Ok(Some(quote(&name)))
            }
            "noise" => {
                // noise KIND POINTS [octaves= lacunarity= H= offset= gain=]: the shader nodes' noise
                // at each point (x y z triples), for mathutils.noise.
                cmd.max_args(2)?;
                let kind = cmd.arg(0, "a noise kind")?.text.clone();
                let pts = floats(&cmd.arg(1, "points")?.text)?;
                let octaves = cmd.f("octaves")?.unwrap_or(1.0).max(1.0);
                let lacunarity = cmd.f("lacunarity")?.unwrap_or(2.0);
                let h = cmd.f("H")?.unwrap_or(1.0);
                let offset = cmd.f("offset")?.unwrap_or(0.0);
                let gain = cmd.f("gain")?.unwrap_or(1.0);
                use crate::render::tex_noise as tex;
                let f = tex::Fractal { detail: octaves - 1.0, roughness: lacunarity.powf(-h), lacunarity, offset, gain, normalize: false };
                let out: Vec<f32> = pts
                    .chunks_exact(3)
                    .map(|c| {
                        let p = [c[0], c[1], c[2]];
                        match kind.as_str() {
                            "perlin" => tex::snoise(p),
                            "turbulence" => {
                                let (mut amp, mut s, mut fq) = (1.0f32, 0.0f32, 1.0f32);
                                for _ in 0..octaves as i32 {
                                    s += amp * tex::snoise(p.map(|x| x * fq)).abs();
                                    amp *= 0.5;
                                    fq *= 2.0;
                                }
                                s
                            }
                            other => tex::noise_select(p, &f, tex::NoiseType::from_name(&other.to_ascii_uppercase().replace("MULTI_FRACTAL", "MULTIFRACTAL"))),
                        }
                    })
                    .collect();
                reply(json!(out))
            }
            "image_pixels" => {
                cmd.max_args(2)?;
                let n = cmd.arg(0, "an image")?.text.clone();
                let v = floats(&cmd.arg(1, "pixels")?.text)?;
                let img = scene.images.iter_mut().find(|x| x.name == n).ok_or_else(|| format!("no image named '{n}'"))?;
                let (w, h) = (img.width as usize, img.height as usize);
                if v.len() != w * h * 4 {
                    return Err(format!("pixels holds {} numbers, the image needs {}", v.len(), w * h * 4));
                }
                for y in 0..h {
                    for x in 0..w {
                        let s = ((h - 1 - y) * w + x) * 4;
                        img.pixels[y * w + x] = [v[s], v[s + 1], v[s + 2], v[s + 3]];
                    }
                }
                Ok(None)
            }
            "image_save" => {
                cmd.max_args(2)?;
                let n = cmd.arg(0, "an image")?.text.clone();
                let path = cmd.arg(1, "a file path")?.text.clone();
                let img = scene.images.iter().find(|x| x.name == n).ok_or_else(|| format!("no image named '{n}'"))?;
                write_png(img, &path)?;
                self.bpy.files_written.push(path);
                Ok(None)
            }
            "image_load" => {
                cmd.max_args(1)?;
                let path = cmd.arg(0, "a file path")?.text.clone();
                let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
                let stem = std::path::Path::new(&path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Image".into());
                let mut img = crate::io::decode_image(&stem, &bytes)?;
                img.name = unique_name(&stem, |n| scene.images.iter().any(|i| i.name == n));
                let name = img.name.clone();
                scene.images.push(img);
                Ok(Some(quote(&name)))
            }
            "use_nodes" => {
                let owner = cmd.arg(0, "material NAME or world")?.text.clone();
                let on_at = if owner == "material" { 2 } else { 1 };
                cmd.max_args(on_at + 1)?;
                let on = boolean(&cmd.arg(on_at, "1 or 0")?.text)?;
                match owner.as_str() {
                    "material" => {
                        let mi = self.material(scene, cmd.arg(1, "a material")?)?;
                        let m = &mut scene.materials[mi];
                        if on && m.node_tree.is_none() {
                            let mut t = crate::nodes::NodeTree::default_material("Shader Nodetree");
                            if let Some(p) = t.nodes.iter_mut().find(|n| n.idname == "ShaderNodeBsdfPrincipled") {
                                let set = |p: &mut crate::nodes::Node, s: &str, v: crate::nodes::Value| {
                                    if let Some(x) = p.inputs.iter_mut().find(|x| x.identifier == s) {
                                        x.value = v;
                                    }
                                };
                                use crate::nodes::Value as V;
                                set(p, "Base Color", V::Color(m.base_color));
                                set(p, "Metallic", V::Float(m.metallic));
                                set(p, "Roughness", V::Float(m.roughness));
                                set(p, "IOR", V::Float(m.ior));
                            }
                            m.node_tree = Some(t);
                        } else if !on {
                            m.node_tree = None;
                        }
                    }
                    "world" => {
                        if on && scene.world.node_tree.is_none() {
                            scene.world.node_tree = Some(crate::nodes::NodeTree::default_world("Shader Nodetree"));
                        } else if !on {
                            scene.world.node_tree = None;
                        }
                    }
                    "compositor" => {
                        if on && scene.compositor.is_none() {
                            // Blender's default: Render Layers -> Composite.
                            let mut t = crate::nodes::NodeTree::new("Compositing Nodetree", crate::nodes::TreeKind::Compositor);
                            if let (Some(a), Some(b)) = (t.add("CompositorNodeRLayers", [-300.0, 300.0]), t.add("CompositorNodeComposite", [200.0, 300.0])) {
                                let _ = t.link_sockets(a, 0, b, 0);
                            }
                            scene.compositor = Some(t);
                        } else if !on {
                            scene.compositor = None;
                        }
                    }
                    o => return Err(format!("use_nodes takes material NAME, world or compositor, not '{o}'")),
                }
                Ok(None)
            }
            "object_copy" => {
                cmd.max_args(1)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let linked = cmd.b("linked")?.unwrap_or(true);
                let name = self.duplicate(scene, i, linked);
                self.bind(cmd, Handle::Object(name.clone()))?;
                Ok(Some(quote(&name)))
            }
            "mesh_copy" => {
                cmd.max_args(1)?;
                let mi = self.mesh(scene, cmd.arg(0, "a mesh")?)?;
                let name = unique_name(&scene.mesh_names[mi], |n| scene.mesh_names.iter().any(|x| x == n));
                let m = scene.meshes[mi].clone();
                let ni = scene.add_mesh(&name, m);
                self.bind(cmd, Handle::Mesh(ni))?;
                Ok(Some(quote(&scene.mesh_names[ni])))
            }
            "material_copy" => {
                cmd.max_args(1)?;
                let mi = self.material(scene, cmd.arg(0, "a material")?)?;
                let mut m = scene.materials[mi].clone();
                m.name = unique_name(&m.name, |n| scene.materials.iter().any(|x| x.name == n));
                let ni = scene.add_material(m);
                self.bind(cmd, Handle::Material(ni))?;
                Ok(Some(quote(&scene.materials[ni].name)))
            }
            "remove" => {
                cmd.max_args(2)?;
                let kind = cmd.arg(0, "a kind")?.text.clone();
                let t = cmd.arg(1, "a name")?.clone();
                self.remove_id(scene, &kind, &t)?;
                Ok(None)
            }
            "rename" => {
                cmd.max_args(3)?;
                let kind = cmd.arg(0, "a kind")?.text.clone();
                let t = cmd.arg(1, "a name")?.clone();
                let new = cmd.arg(2, "a new name")?.text.clone();
                Ok(Some(quote(&self.rename_id(scene, &kind, &t, &new)?)))
            }
            "modifier_move" => {
                cmd.max_args(3)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let from = uint(&cmd.arg(1, "an index")?.text)? as usize;
                let to = uint(&cmd.arg(2, "an index")?.text)? as usize;
                let list = &mut scene.objects[i].modifiers;
                if from >= list.len() || to >= list.len() {
                    return Err(format!("modifier index out of range (the stack has {})", list.len()));
                }
                let m = list.remove(from);
                list.insert(to, m);
                Ok(None)
            }
            "material_slot" => {
                // material_slot OBJ INDEX MAT|none  (set a slot; INDEX may be the count to append)
                // material_slot OBJ INDEX remove=1
                cmd.max_args(3)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let slot = uint(&cmd.arg(1, "a slot index")?.text)? as usize;
                if cmd.b("remove")?.unwrap_or(false) {
                    let o = &mut scene.objects[i];
                    if slot < o.materials.len() {
                        o.materials.remove(slot);
                        if let ObjectData::Mesh(m) = o.data {
                            for f in &mut scene.meshes[m].faces {
                                if f.material as usize > slot || (f.material as usize == slot && slot > 0) {
                                    f.material -= 1;
                                }
                            }
                        }
                    }
                    return Ok(None);
                }
                let t = cmd.arg(2, "a material or none")?.clone();
                // Polyamour's slots always hold a material: an empty slot gets a stand-in.
                let mi = if t.is_word("none") {
                    match scene.materials.iter().position(|m| m.name == "Material") {
                        Some(x) => x,
                        None => scene.add_material(Material { name: "Material".into(), ..Material::default() }),
                    }
                } else {
                    self.material(scene, &t)?
                };
                let o = &mut scene.objects[i];
                if slot < o.materials.len() {
                    o.materials[slot] = mi;
                } else {
                    o.materials.push(mi);
                }
                Ok(None)
            }
            "bone_remove" => {
                cmd.max_args(2)?;
                let t = cmd.arg(0, "an armature")?.clone();
                let ai = self.armature_data_pub(scene, &t)?;
                let bn = cmd.arg(1, "a bone")?.text.clone();
                let arm = &mut scene.armatures[ai];
                let bi = arm.find(&bn).ok_or_else(|| format!("no bone named '{bn}'"))?;
                let parent = arm.bones[bi].parent;
                arm.bones.remove(bi);
                for b in &mut arm.bones {
                    match b.parent {
                        Some(p) if p == bi => {
                            b.parent = parent.map(|q| if q > bi { q - 1 } else { q });
                            b.use_connect = false;
                        }
                        Some(p) if p > bi => b.parent = Some(p - 1),
                        _ => {}
                    }
                }
                for o in &mut scene.objects {
                    if matches!(o.data, ObjectData::Armature(x) if x == ai) {
                        o.pose.retain(|p| p.name != bn);
                    }
                }
                Ok(None)
            }
            n => Err(format!("bpy command {n} is not handled")),
        }
    }

    fn set_mode(&mut self, scene: &mut Scene, mode: &str) -> Res<()> {
        match mode {
            "OBJECT" => {}
            "EDIT" => {
                let a = scene.active_object.ok_or("Edit Mode needs an active object")?;
                if !matches!(scene.objects[a].data, ObjectData::Mesh(_) | ObjectData::Armature(_) | ObjectData::Lattice(_)) {
                    return Err(format!("'{}' has no data to edit", scene.objects[a].name));
                }
            }
            "POSE" => {
                let a = scene.active_object.ok_or("Pose Mode needs an active armature")?;
                if !matches!(scene.objects[a].data, ObjectData::Armature(_)) {
                    return Err(format!("'{}' is not an armature", scene.objects[a].name));
                }
            }
            "SCULPT" | "VERTEX_PAINT" | "WEIGHT_PAINT" | "TEXTURE_PAINT" => {
                let a = scene.active_object.ok_or("this mode needs an active mesh")?;
                if !matches!(scene.objects[a].data, ObjectData::Mesh(_)) {
                    return Err(format!("'{}' is not a mesh", scene.objects[a].name));
                }
            }
            m => return Err(format!("unknown mode '{m}'")),
        }
        self.bpy.mode = mode.to_string();
        Ok(())
    }

    pub(super) fn armature_data_pub(&self, scene: &Scene, t: &Token) -> Res<usize> {
        if let Ok(o) = self.object(scene, t) {
            if let ObjectData::Armature(a) = scene.objects[o].data {
                return Ok(a);
            }
        }
        scene.armature_names.iter().position(|n| *n == t.text).ok_or_else(|| format!("no armature named '{}'", t.text))
    }

    /// Duplicate object `i` (sharing its mesh when `linked`); returns the new name.
    fn duplicate(&mut self, scene: &mut Scene, i: usize, linked: bool) -> String {
        let mut o = scene.objects[i].clone();
        o.name = scene.unique_object_name(&o.name);
        if !linked {
            if let ObjectData::Mesh(m) = o.data {
                let name = unique_name(&scene.mesh_names[m], |n| scene.mesh_names.iter().any(|x| x == n));
                let mesh = scene.meshes[m].clone();
                o.data = ObjectData::Mesh(scene.add_mesh(&name, mesh));
            }
        }
        let name = o.name.clone();
        let ni = scene.add_object(o);
        self.report.created.push(name.clone());
        if let Some(a) = self.bpy.anim.get(&scene.objects[i].name).cloned() {
            self.bpy.anim.insert(name.clone(), a);
        }
        if let Some(m) = self.bpy.rotation_modes.get(&scene.objects[i].name).cloned() {
            self.bpy.rotation_modes.insert(name.clone(), m);
        }
        let _ = ni;
        name
    }

    fn remove_id(&mut self, scene: &mut Scene, kind: &str, t: &Token) -> Res<()> {
        match kind {
            "mesh" => {
                let mi = self.mesh(scene, t)?;
                if scene.objects.iter().any(|o| matches!(o.data, ObjectData::Mesh(x) if x == mi)) {
                    // Blender unlinks the users too; Polyamour removes those objects.
                    let users: Vec<usize> = scene.objects.iter().enumerate().filter(|(_, o)| matches!(o.data, ObjectData::Mesh(x) if x == mi)).map(|(i, _)| i).collect();
                    for &u in users.iter().rev() {
                        scene.remove_object(u);
                    }
                }
                scene.meshes.remove(mi);
                scene.mesh_names.remove(mi);
                for o in &mut scene.objects {
                    if let ObjectData::Mesh(x) = &mut o.data {
                        if *x > mi {
                            *x -= 1;
                        }
                    }
                }
                self.bpy.sel.clear();
                self.handles.retain(|_, h| !matches!(h, Handle::Mesh(x) if *x == mi));
                for h in self.handles.values_mut() {
                    if let Handle::Mesh(x) = h {
                        if *x > mi {
                            *x -= 1;
                        }
                    }
                }
            }
            "material" => {
                let mi = self.material(scene, t)?;
                scene.materials.remove(mi);
                for o in &mut scene.objects {
                    o.materials.retain(|&m| m != mi);
                    for m in &mut o.materials {
                        if *m > mi {
                            *m -= 1;
                        }
                    }
                }
                self.handles.retain(|_, h| !matches!(h, Handle::Material(x) if *x == mi));
                for h in self.handles.values_mut() {
                    if let Handle::Material(x) = h {
                        if *x > mi {
                            *x -= 1;
                        }
                    }
                }
            }
            "image" => {
                let i = scene.images.iter().position(|x| x.name == t.text).ok_or_else(|| format!("no image named '{}'", t.text))?;
                scene.images.remove(i);
                for m in &mut scene.materials {
                    match m.base_color_texture {
                        Some(x) if x == i => m.base_color_texture = None,
                        Some(x) if x > i => m.base_color_texture = Some(x - 1),
                        _ => {}
                    }
                }
                match scene.world.hdri {
                    Some(x) if x == i => scene.world.hdri = None,
                    Some(x) if x > i => scene.world.hdri = Some(x - 1),
                    _ => {}
                }
            }
            "action" => {
                let before = self.bpy.actions.len();
                self.bpy.actions.retain(|a| a.name != t.text);
                if before == self.bpy.actions.len() {
                    return Err(format!("no action named '{}'", t.text));
                }
                self.bpy.anim.retain(|_, a| *a != t.text);
            }
            "node_group" => {
                let g = scene.node_groups.iter().position(|x| x.name == t.text).ok_or_else(|| format!("no node group named '{}'", t.text))?;
                scene.node_groups.remove(g);
            }
            "collection" => {
                let c = self.collection(scene, t)?;
                if c == 0 {
                    return Err("the scene's first collection cannot be removed".into());
                }
                let parent = scene.collections[c].parent.unwrap_or(0);
                let parent = if parent > c { parent - 1 } else { parent };
                scene.collections.remove(c);
                let fix = |x: usize| if x == c { parent } else if x > c { x - 1 } else { x };
                for o in &mut scene.objects {
                    o.collection = fix(o.collection);
                }
                for col in &mut scene.collections {
                    col.parent = col.parent.map(fix);
                }
            }
            k => return Err(format!("cannot remove a {k} here")),
        }
        Ok(())
    }

    fn rename_id(&mut self, scene: &mut Scene, kind: &str, t: &Token, new: &str) -> Res<String> {
        match kind {
            "mesh" => {
                let mi = self.mesh(scene, t)?;
                let n = unique_name(new, |n| scene.mesh_names.iter().enumerate().any(|(j, x)| j != mi && x == n));
                scene.mesh_names[mi] = n.clone();
                Ok(n)
            }
            "material" => {
                let mi = self.material(scene, t)?;
                let n = unique_name(new, |n| scene.materials.iter().enumerate().any(|(j, x)| j != mi && x.name == n));
                scene.materials[mi].name = n.clone();
                Ok(n)
            }
            "image" => {
                let i = scene.images.iter().position(|x| x.name == t.text).ok_or_else(|| format!("no image named '{}'", t.text))?;
                let n = unique_name(new, |n| scene.images.iter().enumerate().any(|(j, x)| j != i && x.name == n));
                scene.images[i].name = n.clone();
                Ok(n)
            }
            "armature" => {
                let a = self.armature_data_pub(scene, t)?;
                let n = unique_name(new, |n| scene.armature_names.iter().enumerate().any(|(j, x)| j != a && x == n));
                scene.armature_names[a] = n.clone();
                Ok(n)
            }
            "lattice" => {
                let l = scene.lattice_names.iter().position(|x| *x == t.text).ok_or_else(|| format!("no lattice named '{}'", t.text))?;
                let n = unique_name(new, |n| scene.lattice_names.iter().enumerate().any(|(j, x)| j != l && x == n));
                scene.lattice_names[l] = n.clone();
                Ok(n)
            }
            "action" => {
                let i = self.bpy.actions.iter().position(|a| a.name == t.text).ok_or_else(|| format!("no action named '{}'", t.text))?;
                let n = unique_name(new, |n| self.bpy.actions.iter().enumerate().any(|(j, x)| j != i && x.name == n));
                self.bpy.actions[i].name = n.clone();
                for a in self.bpy.anim.values_mut() {
                    if *a == t.text {
                        *a = n.clone();
                    }
                }
                Ok(n)
            }
            "node_group" => {
                let g = scene.node_groups.iter().position(|x| x.name == t.text).ok_or_else(|| format!("no node group named '{}'", t.text))?;
                let n = unique_name(new, |n| scene.node_groups.iter().enumerate().any(|(j, x)| j != g && x.name == n));
                scene.node_groups[g].name = n.clone();
                Ok(n)
            }
            "bone" => {
                // rename bone ARM/BONE NEW: t is "ARMATURE/BONE".
                let (arm, bone) = t.text.split_once('/').ok_or("bone renames take ARMATURE/BONE")?;
                let a = self.armature_data_pub(scene, &Token { key: None, text: arm.into(), quoted: true })?;
                let ar = &mut scene.armatures[a];
                let bi = ar.find(bone).ok_or_else(|| format!("no bone named '{bone}'"))?;
                let n = unique_name(new, |n| ar.bones.iter().enumerate().any(|(j, x)| j != bi && x.name == n));
                ar.bones[bi].name = n.clone();
                for o in &mut scene.objects {
                    if matches!(o.data, ObjectData::Armature(x) if x == a) {
                        for p in o.pose.iter_mut().filter(|p| p.name == bone) {
                            p.name = n.clone();
                        }
                    }
                }
                Ok(n)
            }
            k => Err(format!("cannot rename a {k} here")),
        }
    }

    /// Run operator `id`; false means CANCELLED.
    fn run_op(&mut self, scene: &mut Scene, cmd: &mut Cmd, id: &str) -> Res<bool> {
        let (cat, name) = id.split_once('.').unwrap_or(("", id));
        match cat {
            "mesh" if name.starts_with("primitive_") => self.op_primitive(scene, cmd, name),
            "mesh" => self.op_mesh(scene, cmd, name),
            "object" => self.op_object(scene, cmd, name),
            "transform" => self.op_transform(scene, cmd, name),
            "uv" => self.op_uv(scene, cmd, name),
            "pose" | "armature" => self.op_rig(scene, cmd, name),
            "anim" => self.op_anim(scene, cmd, name),
            "screen" => {
                let delta = match name {
                    "frame_jump" => {
                        let end = cmd.b("end")?.unwrap_or(false);
                        let f = if end { scene.frame_end } else { scene.frame_start };
                        f - scene.frame_current
                    }
                    _ => cmd.f("delta")?.unwrap_or(1.0) as i32,
                };
                scene.frame_current += delta;
                let f = scene.frame_current as f32;
                self.bpy.evaluate(scene, f);
                Ok(true)
            }
            "render" => self.op_render(scene, cmd),
            _ => self.op_file(scene, cmd, id),
        }
    }

    fn op_primitive(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        use crate::primitives as p;
        let size = cmd.f("size")?.unwrap_or(2.0);
        let radius = cmd.f("radius")?.unwrap_or(1.0);
        let verts = cmd.u("vertices")?.unwrap_or(32).clamp(3, 4096);
        let depth = cmd.f("depth")?.unwrap_or(2.0);
        let (mesh, base) = match name {
            "primitive_cube_add" => (p::cube(size), "Cube"),
            "primitive_plane_add" => (p::plane(size), "Plane"),
            "primitive_grid_add" => (p::grid(cmd.u("x_subdivisions")?.unwrap_or(10).clamp(1, 4096), cmd.u("y_subdivisions")?.unwrap_or(10).clamp(1, 4096), size), "Grid"),
            "primitive_uv_sphere_add" => (p::uv_sphere(cmd.u("segments")?.unwrap_or(32).clamp(3, 4096), cmd.u("ring_count")?.unwrap_or(16).clamp(3, 4096), radius), "Sphere"),
            "primitive_ico_sphere_add" => (p::ico_sphere(cmd.u("subdivisions")?.unwrap_or(2).min(8), radius), "Icosphere"),
            "primitive_cylinder_add" => (p::cylinder(verts, radius, depth), "Cylinder"),
            "primitive_cone_add" => (p::cone(verts, cmd.f("radius1")?.unwrap_or(1.0), cmd.f("radius2")?.unwrap_or(0.0), depth), "Cone"),
            "primitive_torus_add" => (
                p::torus(
                    cmd.u("major_segments")?.unwrap_or(48).clamp(3, 4096),
                    cmd.u("minor_segments")?.unwrap_or(12).clamp(3, 4096),
                    cmd.f("major_radius")?.unwrap_or(1.0),
                    cmd.f("minor_radius")?.unwrap_or(0.25),
                ),
                "Torus",
            ),
            "primitive_monkey_add" => {
                let mut m = p::monkey();
                if size != 2.0 {
                    m.transform(&Mat4::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(size / 2.0)));
                }
                (m, "Suzanne")
            }
            "primitive_circle_add" => {
                let mut m = Mesh::new();
                for k in 0..verts {
                    let a = k as f32 / verts as f32 * std::f32::consts::TAU;
                    m.positions.push(Vec3::new(radius * a.cos(), radius * a.sin(), 0.0));
                }
                match cmd.opt("fill_type").map(|t| t.text.to_ascii_uppercase()).as_deref() {
                    Some("NGON") => m.faces.push(Face::new((0..verts).collect())),
                    Some("TRIFAN") => {
                        let c = m.add_vertex(Vec3::ZERO);
                        for k in 0..verts {
                            m.faces.push(Face::new(vec![c, k, (k + 1) % verts]));
                        }
                    }
                    _ => m.loose_edges = (0..verts).map(|k| [k, (k + 1) % verts]).collect(),
                }
                (m, "Circle")
            }
            _ => return Err(format!("unknown primitive operator {name}")),
        };
        let edit = self.bpy.mode == "EDIT" && scene.active_object.is_some_and(|a| matches!(scene.objects[a].data, ObjectData::Mesh(_)));
        let loc = cmd.v3("location")?.unwrap_or(Vec3::from_array(self.bpy.cursor));
        let rot = cmd.v3("rotation")?.unwrap_or(Vec3::ZERO);
        let scale = cmd.v3("scale")?.unwrap_or(Vec3::ONE);
        if edit {
            // In Edit Mode a primitive joins the edited mesh, selected alone.
            let a = scene.active_object.expect("checked");
            let ObjectData::Mesh(mi) = scene.objects[a].data else { unreachable!() };
            let local = scene.world_matrix(a).inverse() * Mat4::from_trs(loc, Quat::from_euler_xyz(rot.x, rot.y, rot.z), scale);
            let mut add = mesh;
            add.transform(&local);
            let m = &mut scene.meshes[mi];
            let base_v = m.positions.len() as u32;
            let base_f = m.faces.len() as u32;
            m.append(&add);
            let s = Sel { verts: (base_v..m.positions.len() as u32).collect(), edges: BTreeSet::new(), faces: (base_f..m.faces.len() as u32).collect() };
            self.bpy.sel.insert(mi, s);
            return Ok(true);
        }
        let i = scene.add_mesh_object(base, mesh);
        let o = &mut scene.objects[i];
        o.transform.location = loc;
        o.transform.rotation = Quat::from_euler_xyz(rot.x, rot.y, rot.z);
        o.transform.scale = scale;
        if let Some(t) = cmd.opt("collection") {
            scene.objects[i].collection = self.collection(scene, &t)?;
        }
        let name = scene.objects[i].name.clone();
        if rot != Vec3::ZERO {
            self.bpy.eulers.insert(name.clone(), rot.to_array());
        }
        scene.active_object = Some(i);
        scene.selected = vec![i];
        self.report.created.push(name);
        if cmd.b("enter_editmode")?.unwrap_or(false) {
            self.bpy.mode = "EDIT".into();
            let ObjectData::Mesh(mi) = scene.objects[i].data else { unreachable!() };
            self.bpy.sel.insert(mi, Sel::all(&scene.meshes[mi]));
        }
        Ok(true)
    }

    fn op_mesh(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        use crate::ops;
        use crate::ops2;
        let mi = self.edit_mesh(scene)?;
        let sel = self.bpy.sel.get(&mi).cloned().unwrap_or_default();
        let m = &mut scene.meshes[mi];
        let (faces, edges, verts) = (sel.faces(m), sel.edges(m), sel.verts(m));
        let mut new_sel: Option<Sel> = None;
        let translate = |m: &mut Mesh, s: &Sel, v: Option<Vec3>| {
            if let Some(d) = v {
                for &i in &s.verts(m) {
                    m.positions[i as usize] += d;
                }
            }
        };
        match name {
            "select_all" => {
                let action = cmd.opt("action").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "TOGGLE".into());
                let any = !sel.is_empty();
                new_sel = Some(match action.as_str() {
                    "SELECT" => Sel::all(m),
                    "DESELECT" => Sel::default(),
                    "INVERT" => {
                        let mut s = Sel::default();
                        let fs: BTreeSet<u32> = faces.iter().copied().collect();
                        let vs: BTreeSet<u32> = verts.iter().copied().collect();
                        s.verts = (0..m.positions.len() as u32).filter(|v| !vs.contains(v)).collect();
                        s.faces = (0..m.faces.len() as u32).filter(|f| !fs.contains(f)).collect();
                        s
                    }
                    _ if any => Sel::default(),
                    _ => Sel::all(m),
                });
            }
            "select_mode" => {
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "VERT".into());
                self.bpy.select_mode = [ty == "VERT", ty == "EDGE", ty == "FACE"];
            }
            "select_non_manifold" => new_sel = Some({ let r = ops2::non_manifold(m); Sel::from_selection(m, &r) }),
            "select_linked" | "select_more" | "select_less" => {
                let he = m.half_edges();
                let mut s: BTreeSet<u32> = verts.iter().copied().collect();
                match name {
                    "select_linked" => {
                        let mut stack: Vec<u32> = s.iter().copied().collect();
                        while let Some(v) = stack.pop() {
                            for n in he.vertex_neighbors(v) {
                                if s.insert(n) {
                                    stack.push(n);
                                }
                            }
                        }
                    }
                    "select_more" => {
                        for v in verts.clone() {
                            s.extend(he.vertex_neighbors(v));
                        }
                    }
                    _ => {
                        let border: Vec<u32> = verts.iter().copied().filter(|&v| he.vertex_neighbors(v).iter().any(|n| !s.contains(n))).collect();
                        for v in border {
                            s.remove(&v);
                        }
                    }
                }
                new_sel = Some(Sel { verts: s, ..Default::default() });
            }
            "extrude_region_move" | "extrude_region" | "extrude_faces_move" | "extrude_region_shrink_fatten" => {
                let r = if name == "extrude_faces_move" && !faces.is_empty() {
                    let mut all = Selection::default();
                    for &f in &faces {
                        let s = ops2::extrude_geometry(m, &[f], &[], &[]);
                        all.verts.extend(s.verts);
                        all.edges.extend(s.edges);
                        all.faces.extend(s.faces);
                    }
                    all
                } else {
                    ops2::extrude_geometry(m, &faces, &edges, &verts)
                };
                let s = { let r = r; Sel::from_selection(m, &r) };
                let value = cmd.v3("value")?;
                if name == "extrude_region_shrink_fatten" {
                    if let Some(d) = cmd.f("shrink_fatten")?.or_else(|| value.map(|v| v.x)) {
                        let n = s.faces(m).iter().fold(Vec3::ZERO, |a, &f| a + m.face_normal(f as usize)).normalize();
                        translate(m, &s, Some(n * d));
                    }
                } else {
                    translate(m, &s, value);
                }
                new_sel = Some(s);
            }
            "extrude_repeat" => {
                let steps = cmd.u("steps")?.unwrap_or(10).min(1000);
                let off = cmd.v3("offset")?.unwrap_or(Vec3::new(0.0, 0.0, 1.0));
                new_sel = Some({ let r = ops2::extrude_repeat(m, &faces, &edges, &verts, off, steps); Sel::from_selection(m, &r) });
            }
            "subdivide" => {
                let cuts = cmd.u("number_cuts")?.unwrap_or(1).clamp(1, 100);
                new_sel = Some({ let r = ops2::subdivide(m, &edges, cuts); Sel::from_selection(m, &r) });
            }
            "delete" => {
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "VERT".into());
                match ty.as_str() {
                    "VERT" => ops::delete_verts(m, &verts),
                    "FACE" | "ONLY_FACE" => {
                        ops::delete_faces(m, &faces);
                        if ty == "FACE" {
                            m.remove_unused_vertices();
                        }
                    }
                    "EDGE" => {
                        let es: BTreeSet<(u32, u32)> = edges.iter().map(|e| (e[0].min(e[1]), e[0].max(e[1]))).collect();
                        let fs: Vec<u32> = (0..m.faces.len() as u32).filter(|&f| m.faces[f as usize].edges().any(|(a, b)| es.contains(&(a.min(b), a.max(b))))).collect();
                        ops::delete_faces(m, &fs);
                        m.loose_edges.retain(|e| !es.contains(&(e[0].min(e[1]), e[0].max(e[1]))));
                        m.remove_unused_vertices();
                    }
                    "EDGE_FACE" => {
                        ops::delete_faces(m, &faces);
                    }
                    t => return Err(format!("delete type '{t}' is not VERT, EDGE, FACE or ONLY_FACE")),
                }
                new_sel = Some(Sel::default());
            }
            "remove_doubles" => {
                let d = cmd.f("threshold")?.unwrap_or(0.0001);
                let n = if verts.len() == m.positions.len() {
                    m.weld(d)
                } else {
                    // Only selected vertices merge: weld a copy restricted to them.
                    let set: BTreeSet<u32> = verts.iter().copied().collect();
                    let mut remap: Vec<u32> = (0..m.positions.len() as u32).collect();
                    let list: Vec<u32> = set.iter().copied().collect();
                    let mut n = 0;
                    for (k, &a) in list.iter().enumerate() {
                        if remap[a as usize] != a {
                            continue;
                        }
                        for &b in &list[k + 1..] {
                            if remap[b as usize] == b && m.positions[a as usize].distance(m.positions[b as usize]) <= d {
                                remap[b as usize] = a;
                                n += 1;
                            }
                        }
                    }
                    m.remap_vertices(&remap);
                    m.remove_unused_vertices();
                    n
                };
                self.bpy.reports.push(("INFO".into(), format!("Removed {n} vertice(s)")));
                new_sel = Some(Sel::default());
            }
            "merge" => {
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "CENTER".into());
                let r = match ty.as_str() {
                    "FIRST" | "LAST" if !verts.is_empty() => ops2::merge_at_vertex(m, &verts, if ty == "FIRST" { verts[0] } else { *verts.last().expect("non-empty") }),
                    _ => ops::merge_at_center(m, &verts),
                };
                new_sel = Some(Sel { verts: r.into_iter().collect(), ..Default::default() });
            }
            "normals_make_consistent" => {
                ops::recalc_normals_outside(m);
                if cmd.b("inside")?.unwrap_or(false) {
                    let all: Vec<u32> = (0..m.faces.len() as u32).collect();
                    ops::flip_normals(m, &all);
                }
            }
            "flip_normals" => ops::flip_normals(m, &faces),
            "inset" => {
                let th = cmd.f("thickness")?.unwrap_or(0.0);
                let dp = cmd.f("depth")?.unwrap_or(0.0);
                let r = if cmd.b("use_individual")?.unwrap_or(false) { ops::inset_faces_individual(m, &faces, th, dp) } else { ops::inset_region(m, &faces, th, dp) };
                let _ = r;
                new_sel = Some(Sel { faces: faces.iter().copied().collect(), ..Default::default() });
            }
            "bevel" => {
                let w = cmd.f("offset")?.or(cmd.f("width")?).unwrap_or(0.0);
                let seg = cmd.u("segments")?.unwrap_or(1).clamp(1, 100);
                let f = ops::bevel_edges(m, &edges, w, seg);
                new_sel = Some(Sel { faces: f.into_iter().collect(), ..Default::default() });
            }
            "quads_convert_to_tris" => {
                let fs: BTreeSet<u32> = faces.iter().copied().collect();
                let mut out = Vec::new();
                let mut picked = Vec::new();
                for (fi, f) in m.faces.iter().enumerate() {
                    if fs.contains(&(fi as u32)) && f.verts.len() > 3 {
                        for k in 1..f.verts.len() - 1 {
                            let mut t = f.clone();
                            t.verts = vec![f.verts[0], f.verts[k], f.verts[k + 1]];
                            if !f.uvs.is_empty() {
                                t.uvs = vec![f.uvs[0], f.uvs[k], f.uvs[k + 1]];
                            }
                            picked.push(out.len() as u32);
                            out.push(t);
                        }
                    } else {
                        if fs.contains(&(fi as u32)) {
                            picked.push(out.len() as u32);
                        }
                        out.push(f.clone());
                    }
                }
                m.faces = out;
                new_sel = Some(Sel { faces: picked.into_iter().collect(), ..Default::default() });
            }
            "tris_convert_to_quads" => {
                let a = cmd.f("face_threshold")?.unwrap_or(40f32.to_radians());
                let b = cmd.f("shape_threshold")?.unwrap_or(40f32.to_radians());
                new_sel = Some({ let r = ops2::tris_to_quads(m, &faces, a, b); Sel::from_selection(m, &r) });
            }
            "fill" | "edge_face_add" => {
                if name == "edge_face_add" && edges.is_empty() && verts.len() >= 3 {
                    let f = m.add_face(&verts);
                    new_sel = Some(Sel { faces: [f].into_iter().collect(), ..Default::default() });
                } else {
                    new_sel = Some({ let r = ops2::fill(m, &edges, cmd.b("use_beauty")?.unwrap_or(true)); Sel::from_selection(m, &r) });
                }
            }
            "fill_grid" => new_sel = Some({ let r = ops2::fill_grid(m, &edges)?; Sel::from_selection(m, &r) }),
            "beautify_fill" => new_sel = Some({ let r = ops2::beautify_fill(m, &faces); Sel::from_selection(m, &r) }),
            "poke" => new_sel = Some({ let r = ops2::poke(m, &faces, cmd.f("offset")?.unwrap_or(0.0)); Sel::from_selection(m, &r) }),
            "dissolve_verts" => new_sel = Some({ let r = ops2::dissolve_verts(m, &verts); Sel::from_selection(m, &r) }),
            "dissolve_edges" => new_sel = Some({ let r = ops2::dissolve_edges(m, &edges, cmd.b("use_verts")?.unwrap_or(true)); Sel::from_selection(m, &r) }),
            "dissolve_faces" => new_sel = Some({ let r = ops2::dissolve_faces(m, &faces, cmd.b("use_verts")?.unwrap_or(false)); Sel::from_selection(m, &r) }),
            "dissolve_limited" => {
                ops::dissolve_coplanar(m, cmd.f("angle_limit")?.unwrap_or(5f32.to_radians()));
                new_sel = Some(Sel::default());
            }
            "unsubdivide" => new_sel = Some({ let r = ops2::unsubdivide(m, &verts, cmd.u("iterations")?.unwrap_or(2).min(10)); Sel::from_selection(m, &r) }),
            "edge_split" => new_sel = Some({ let r = ops2::edge_split(m, &edges); Sel::from_selection(m, &r) }),
            "split" => new_sel = Some({ let r = ops2::split(m, &faces); Sel::from_selection(m, &r) }),
            "spin" => {
                let steps = cmd.u("steps")?.unwrap_or(12).clamp(1, 1000);
                let angle = cmd.f("angle")?.unwrap_or(std::f32::consts::TAU);
                let center = cmd.v3("center")?.unwrap_or(Vec3::ZERO);
                let axis = cmd.v3("axis")?.unwrap_or(Vec3::new(0.0, 0.0, 1.0));
                new_sel = Some({ let r = ops2::spin(m, &faces, &edges, &verts, center, axis, angle, steps, 0.0); Sel::from_selection(m, &r) });
            }
            "bisect" => {
                let co = cmd.v3("plane_co")?.unwrap_or(Vec3::ZERO);
                let no = cmd.v3("plane_no")?.unwrap_or(Vec3::new(0.0, 0.0, 1.0));
                let fill = cmd.b("use_fill")?.unwrap_or(false);
                let inner = cmd.b("clear_inner")?.unwrap_or(false);
                let outer = cmd.b("clear_outer")?.unwrap_or(false);
                let v = ops::bisect(m, co, no, fill, inner, outer);
                new_sel = Some(Sel { verts: v.into_iter().collect(), ..Default::default() });
            }
            "convex_hull" => new_sel = Some({ let r = ops2::convex_hull(m, &verts, &faces)?; Sel::from_selection(m, &r) }),
            "symmetrize" => {
                let dir = cmd.opt("direction").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "NEGATIVE_X".into());
                let axis = if dir.ends_with('Y') { 1 } else if dir.ends_with('Z') { 2 } else { 0 };
                let thr = cmd.f("threshold")?.unwrap_or(0.0001);
                new_sel = Some({ let r = ops2::symmetrize(m, &faces, axis, dir.starts_with("NEGATIVE"), thr); Sel::from_selection(m, &r) });
            }
            "mark_seam" | "mark_sharp" => {
                let clear = cmd.b("clear")?.unwrap_or(false);
                for e in &edges {
                    if name == "mark_seam" {
                        m.set_seam(e[0], e[1], !clear);
                    } else {
                        m.set_sharp(e[0], e[1], !clear);
                    }
                }
            }
            "faces_shade_smooth" | "faces_shade_flat" => {
                for &f in &faces {
                    m.faces[f as usize].smooth = name == "faces_shade_smooth";
                }
            }
            "loopcut" | "loopcut_slide" => {
                let cuts = cmd.u("number_cuts")?.unwrap_or(1).clamp(1, 100);
                let all = m.edges();
                let e = cmd.u("edge_index")?.map(|i| i as usize).filter(|&i| i < all.len()).map(|i| all[i]).or_else(|| edges.first().copied()).ok_or("loopcut needs edge_index or a selected edge")?;
                let v = ops::loop_cut(m, e, cuts);
                new_sel = Some(Sel { verts: v.into_iter().collect(), ..Default::default() });
            }
            "duplicate" | "duplicate_move" => {
                let fs: Vec<u32> = faces.clone();
                let mut copy = Mesh::new();
                let mut map: HashMap<u32, u32> = HashMap::new();
                for &f in &fs {
                    let face = m.faces[f as usize].clone();
                    let mut nf = face.clone();
                    nf.verts = face.verts.iter().map(|&v| *map.entry(v).or_insert_with(|| copy.add_vertex(m.positions[v as usize]))).collect();
                    copy.faces.push(nf);
                }
                let base_v = m.positions.len() as u32;
                let base_f = m.faces.len() as u32;
                m.append(&copy);
                let s = Sel { verts: (base_v..m.positions.len() as u32).collect(), faces: (base_f..m.faces.len() as u32).collect(), ..Default::default() };
                translate(m, &s, cmd.v3("value")?);
                new_sel = Some(s);
            }
            "reveal" | "hide" => {}
            _ => return Err(format!("mesh.{name} is not implemented")),
        }
        m.prune_flags();
        if let Some(s) = new_sel {
            self.bpy.sel.insert(mi, s);
        }
        Ok(true)
    }

    fn op_object(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        let active = scene.active_object;
        let targets = Self::targets(scene);
        let modifier_index = |cmd: &mut Cmd, scene: &Scene| -> Res<(usize, usize)> {
            let a = active.ok_or("no active object")?;
            let list = &scene.objects[a].modifiers;
            let m = cmd.opt("modifier").ok_or("this operator needs modifier=")?;
            let i = match m.text.parse::<usize>() {
                Ok(i) if i < list.len() => i,
                _ => list.iter().position(|x| x.name() == m.text || x.type_name() == m.text).ok_or_else(|| format!("no modifier named '{}'", m.text))?,
            };
            Ok((a, i))
        };
        match name {
            "select_all" => {
                let action = cmd.opt("action").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "TOGGLE".into());
                scene.selected = match action.as_str() {
                    "SELECT" => (0..scene.objects.len()).collect(),
                    "DESELECT" => Vec::new(),
                    "INVERT" => (0..scene.objects.len()).filter(|i| !scene.selected.contains(i)).collect(),
                    _ if scene.selected.is_empty() => (0..scene.objects.len()).collect(),
                    _ => Vec::new(),
                };
            }
            "select_by_type" => {
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "MESH".into());
                if !cmd.b("extend")?.unwrap_or(false) {
                    scene.selected.clear();
                }
                for (i, o) in scene.objects.iter().enumerate() {
                    if object_type(o) == ty && !scene.selected.contains(&i) {
                        scene.selected.push(i);
                    }
                }
            }
            "delete" => {
                let mut t = targets;
                t.sort_unstable();
                for &i in t.iter().rev() {
                    let n = scene.objects[i].name.clone();
                    scene.remove_object(i);
                    self.handles.retain(|_, h| !matches!(h, Handle::Object(x) if *x == n));
                    self.bpy.anim.remove(&n);
                }
            }
            "duplicate" | "duplicate_move" | "duplicate_move_linked" => {
                let linked = name == "duplicate_move_linked" || cmd.b("linked")?.unwrap_or(false);
                let d = cmd.v3("value")?;
                let mut made = Vec::new();
                for i in targets {
                    let n = self.duplicate(scene, i, linked);
                    let ni = scene.find_object(&n).expect("just added");
                    if let Some(d) = d {
                        scene.objects[ni].transform.location += d;
                    }
                    made.push(ni);
                }
                scene.active_object = made.last().copied();
                scene.selected = made;
            }
            "join" => {
                let a = active.ok_or("no active object")?;
                let ObjectData::Mesh(am) = scene.objects[a].data else { return Err("join needs an active mesh".into()) };
                let inv = scene.world_matrix(a).inverse();
                let mut others: Vec<usize> = scene.selected.iter().copied().filter(|&i| i != a && matches!(scene.objects[i].data, ObjectData::Mesh(_))).collect();
                for &i in &others {
                    let ObjectData::Mesh(mi) = scene.objects[i].data else { continue };
                    let mut m = scene.meshes[mi].clone();
                    m.transform(&(inv * scene.world_matrix(i)));
                    // Material slots: map the other object's slots onto the active's.
                    let slots: Vec<u16> = scene.objects[i]
                        .materials
                        .clone()
                        .into_iter()
                        .map(|mat| {
                            let list = &mut scene.objects[a].materials;
                            match list.iter().position(|&x| x == mat) {
                                Some(p) => p as u16,
                                None => {
                                    list.push(mat);
                                    (list.len() - 1) as u16
                                }
                            }
                        })
                        .collect();
                    for f in &mut m.faces {
                        f.material = slots.get(f.material as usize).copied().unwrap_or(0);
                    }
                    scene.meshes[am].append(&m);
                }
                others.sort_unstable();
                for &i in others.iter().rev() {
                    scene.remove_object(i);
                }
                let a = scene.objects.iter().position(|o| matches!(o.data, ObjectData::Mesh(x) if x == am)).unwrap_or(0);
                scene.active_object = Some(a);
                scene.selected = vec![a];
            }
            "shade_smooth" | "shade_flat" | "shade_auto_smooth" => {
                for i in targets {
                    if let ObjectData::Mesh(m) = scene.objects[i].data {
                        for f in &mut scene.meshes[m].faces {
                            f.smooth = name != "shade_flat";
                        }
                    }
                }
            }
            "modifier_add" => {
                let a = active.ok_or("no active object")?;
                let ty = cmd.opt("type").map(|t| t.text).unwrap_or_else(|| "SUBSURF".into());
                let m = default_modifier(&ty)?;
                scene.objects[a].modifiers.push(m);
            }
            "modifier_remove" => {
                let (a, i) = modifier_index(cmd, scene)?;
                scene.objects[a].modifiers.remove(i);
            }
            "modifier_apply" => {
                let (a, i) = modifier_index(cmd, scene)?;
                let ObjectData::Mesh(mi) = scene.objects[a].data else { return Err("modifier_apply needs a mesh object".into()) };
                if scene.objects.iter().filter(|o| matches!(o.data, ObjectData::Mesh(x) if x == mi)).count() > 1 {
                    return Err("Modifiers cannot be applied to multi-user data".into());
                }
                let stack = vec![scene.objects[a].modifiers[i].clone()];
                let base = scene.meshes[mi].clone();
                let out = scene.apply_modifiers(a, &base, &stack);
                scene.meshes[mi] = out;
                scene.objects[a].modifiers.remove(i);
                self.bpy.sel.remove(&mi);
            }
            "modifier_move_up" | "modifier_move_down" | "modifier_move_to_index" => {
                let (a, i) = modifier_index(cmd, scene)?;
                let n = scene.objects[a].modifiers.len();
                let to = match name {
                    "modifier_move_up" => i.saturating_sub(1),
                    "modifier_move_down" => (i + 1).min(n - 1),
                    _ => (cmd.u("index")?.unwrap_or(0) as usize).min(n - 1),
                };
                let m = scene.objects[a].modifiers.remove(i);
                scene.objects[a].modifiers.insert(to, m);
            }
            "transform_apply" => {
                let (l, r, s) = (cmd.b("location")?.unwrap_or(true), cmd.b("rotation")?.unwrap_or(true), cmd.b("scale")?.unwrap_or(true));
                for i in targets {
                    let t = scene.objects[i].transform;
                    let mut keep = t;
                    let mut bake = Transform::default();
                    if l {
                        bake.location = t.location;
                        keep.location = Vec3::ZERO;
                    }
                    if r {
                        bake.rotation = t.rotation;
                        keep.rotation = Quat::IDENTITY;
                    }
                    if s {
                        bake.scale = t.scale;
                        keep.scale = Vec3::ONE;
                    }
                    // Baked = keep^-1 * full, applied in the object's local space.
                    let m = keep.matrix().inverse() * t.matrix();
                    let _ = bake;
                    match scene.objects[i].data {
                        ObjectData::Mesh(mi) => scene.meshes[mi].transform(&m),
                        _ => continue,
                    }
                    scene.objects[i].transform = keep;
                    self.bpy.eulers.remove(&scene.objects[i].name);
                }
            }
            "origin_set" => {
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "ORIGIN_GEOMETRY".into());
                let center = cmd.opt("center").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "MEDIAN".into());
                for i in targets {
                    let ObjectData::Mesh(mi) = scene.objects[i].data else { continue };
                    let world = scene.world_matrix(i);
                    let m = &mut scene.meshes[mi];
                    if m.positions.is_empty() {
                        continue;
                    }
                    let geo_center = if center == "BOUNDS" {
                        m.bounds().center()
                    } else {
                        m.positions.iter().fold(Vec3::ZERO, |a, p| a + *p) / m.positions.len() as f32
                    };
                    match ty.as_str() {
                        "GEOMETRY_ORIGIN" => m.translate(-geo_center),
                        "ORIGIN_GEOMETRY" | "ORIGIN_CURSOR" | "ORIGIN_CENTER_OF_MASS" | "ORIGIN_CENTER_OF_VOLUME" => {
                            let local = if ty == "ORIGIN_CURSOR" { world.inverse().transform_point(Vec3::from_array(self.bpy.cursor)) } else { geo_center };
                            m.translate(-local);
                            let shift = world.transform_vector(local);
                            let o = &mut scene.objects[i];
                            o.transform.location += if o.parent.is_some() { o.transform.rotation.rotate(local.mul_elem(o.transform.scale)) } else { shift };
                        }
                        t => return Err(format!("origin_set type '{t}' is not supported")),
                    }
                }
            }
            "parent_set" => {
                let a = active.ok_or("no active object")?;
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "OBJECT".into());
                let keep = cmd.b("keep_transform")?.unwrap_or(false);
                let children: Vec<usize> = scene.selected.iter().copied().filter(|&i| i != a).collect();
                for c in children {
                    match ty.as_str() {
                        "ARMATURE" | "ARMATURE_NAME" | "ARMATURE_AUTO" | "ARMATURE_ENVELOPE" => {
                            let w = match ty.as_str() {
                                "ARMATURE_NAME" => ArmatureWeights::Empty,
                                "ARMATURE_AUTO" => ArmatureWeights::Automatic,
                                "ARMATURE_ENVELOPE" => ArmatureWeights::Envelope,
                                _ => ArmatureWeights::None,
                            };
                            if !scene.parent_with_armature(c, a, w) {
                                return Err("parenting with an armature failed".into());
                            }
                        }
                        "BONE" => {
                            let bone = self.bpy.bone_sel.get(&scene.objects[a].name).and_then(|s| s.iter().next().cloned()).or_else(|| scene.armature_of(a).and_then(|ar| ar.bones.first()).map(|b| b.name.clone())).ok_or("no bone to parent to")?;
                            if !scene.parent_to_bone(c, a, &bone, true) {
                                return Err("parenting to the bone failed".into());
                            }
                        }
                        _ => {
                            let mut p = Some(a);
                            while let Some(x) = p {
                                if x == c {
                                    return Err("Loop in parents".into());
                                }
                                p = scene.objects[x].parent;
                            }
                            let world = scene.world_matrix(c);
                            scene.objects[c].parent = Some(a);
                            scene.objects[c].parent_type = crate::scene::ParentType::Object;
                            // Blender's Ctrl+P keeps the world transform (parent inverse); Polyamour
                            // has no parent inverse, so the local transform absorbs it.
                            let _ = keep;
                            scene.objects[c].transform = Transform::from_matrix(&(scene.world_matrix(a).inverse() * world));
                        }
                    }
                }
            }
            "parent_clear" => {
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "CLEAR".into());
                for i in targets {
                    if ty == "CLEAR_INVERSE" {
                        continue;
                    }
                    let world = scene.world_matrix(i);
                    scene.objects[i].parent = None;
                    scene.objects[i].parent_type = crate::scene::ParentType::Object;
                    if ty == "CLEAR_KEEP_TRANSFORM" {
                        scene.objects[i].transform = Transform::from_matrix(&world);
                    }
                }
            }
            "convert" => {
                for i in targets {
                    if let Some(m) = scene.evaluated_mesh(i) {
                        if let ObjectData::Mesh(mi) = scene.objects[i].data {
                            scene.meshes[mi] = m;
                            scene.objects[i].modifiers.clear();
                        }
                    }
                }
            }
            "location_clear" | "rotation_clear" | "scale_clear" => {
                for i in targets {
                    let t = &mut scene.objects[i].transform;
                    match name {
                        "location_clear" => t.location = Vec3::ZERO,
                        "rotation_clear" => t.rotation = Quat::IDENTITY,
                        _ => t.scale = Vec3::ONE,
                    }
                    self.bpy.eulers.remove(&scene.objects[i].name);
                }
            }
            "hide_view_set" => {
                let unselected = cmd.b("unselected")?.unwrap_or(false);
                for i in 0..scene.objects.len() {
                    if scene.selected.contains(&i) != unselected {
                        scene.objects[i].visible = false;
                    }
                }
                if !unselected {
                    scene.selected.clear();
                }
            }
            "hide_view_clear" => {
                for o in &mut scene.objects {
                    o.visible = true;
                }
            }
            "material_slot_add" => {
                let a = active.ok_or("no active object")?;
                let mi = match scene.materials.iter().position(|m| m.name == "Material") {
                    Some(x) => x,
                    None => scene.add_material(Material { name: "Material".into(), ..Material::default() }),
                };
                scene.objects[a].materials.push(mi);
            }
            "material_slot_remove" => {
                let a = active.ok_or("no active object")?;
                scene.objects[a].materials.pop();
            }
            "vertex_group_add" | "vertex_group_remove" => {
                let a = active.ok_or("no active object")?;
                let ObjectData::Mesh(mi) = scene.objects[a].data else { return Ok(false) };
                let m = &mut scene.meshes[mi];
                if name == "vertex_group_add" {
                    let n = unique_name("Group", |n| m.vertex_groups.iter().any(|g| g == n));
                    m.add_vertex_group(&n);
                } else if !m.vertex_groups.is_empty() {
                    let g = m.vertex_groups.len() - 1;
                    m.remove_vertex_group(g);
                }
            }
            "shape_key_add" => {
                let a = active.ok_or("no active object")?;
                let ObjectData::Mesh(mi) = scene.objects[a].data else { return Ok(false) };
                let m = &mut scene.meshes[mi];
                let base = if m.shape_keys.is_empty() { "Basis".to_string() } else { "Key".to_string() };
                let n = if base == "Basis" { base } else { unique_name("Key 1", |n| m.shape_keys.iter().any(|k| k.name == n)) };
                m.add_shape_key(&n, cmd.b("from_mix")?.unwrap_or(false));
            }
            "mode_set" => {
                let mode = cmd.opt("mode").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "OBJECT".into());
                self.set_mode(scene, &mode)?;
            }
            "editmode_toggle" => {
                let m = if self.bpy.mode == "EDIT" { "OBJECT" } else { "EDIT" };
                self.set_mode(scene, m)?;
            }
            "posemode_toggle" => {
                let m = if self.bpy.mode == "POSE" { "OBJECT" } else { "POSE" };
                self.set_mode(scene, m)?;
            }
            "light_add" | "camera_add" | "empty_add" | "armature_add" => {
                let loc = cmd.v3("location")?.unwrap_or(Vec3::from_array(self.bpy.cursor));
                let rot = cmd.v3("rotation")?.unwrap_or(Vec3::ZERO);
                let (data, base) = match name {
                    "light_add" => {
                        let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "POINT".into());
                        let kind = match ty.as_str() {
                            "SUN" => LightKind::Sun,
                            "SPOT" => LightKind::Spot { angle: 45f32.to_radians(), blend: 0.15 },
                            "AREA" => LightKind::Area { size: 1.0 },
                            _ => LightKind::Point,
                        };
                        let mut l = Light { kind, ..Light::default() };
                        if let Some(r) = cmd.f("radius")? {
                            l.radius = r;
                        }
                        let n = match kind {
                            LightKind::Sun => "Sun",
                            LightKind::Spot { .. } => "Spot",
                            LightKind::Area { .. } => "Area",
                            LightKind::Point => "Point",
                        };
                        (ObjectData::Light(l), n)
                    }
                    "camera_add" => (ObjectData::Camera(Camera::default()), "Camera"),
                    "armature_add" => {
                        let mut arm = crate::armature::Armature::default();
                        arm.add_bone(Bone::new("Bone", Vec3::ZERO, Vec3::new(0.0, 0.0, cmd.f("radius")?.unwrap_or(1.0))));
                        let an = unique_name("Armature", |n| scene.armature_names.iter().any(|x| x == n));
                        scene.armatures.push(arm);
                        scene.armature_names.push(an);
                        (ObjectData::Armature(scene.armatures.len() - 1), "Armature")
                    }
                    _ => (ObjectData::Empty, "Empty"),
                };
                let n = scene.unique_object_name(base);
                let i = scene.add_object(Object::new(&n, data));
                scene.objects[i].transform.location = loc;
                scene.objects[i].transform.rotation = Quat::from_euler_xyz(rot.x, rot.y, rot.z);
                if name == "camera_add" && scene.active_camera.is_none() {
                    scene.active_camera = Some(i);
                }
                scene.active_object = Some(i);
                scene.selected = vec![i];
                self.report.created.push(n);
            }
            "constraint_add" => {
                let a = active.ok_or("no active object")?;
                let ty = cmd.opt("type").map(|t| t.text).unwrap_or_else(|| "COPY_LOCATION".into());
                let o = scene.objects[a].name.clone();
                let mut sub = Cmd { name: "constraint_add".into(), pos: vec![Token { key: None, text: o, quoted: true }, Token { key: None, text: ty.to_ascii_lowercase(), quoted: false }], opts: Vec::new() };
                self.rig_command(scene, &mut sub)?;
            }
            "make_single_user" => {
                for i in targets {
                    if let ObjectData::Mesh(mi) = scene.objects[i].data {
                        if scene.objects.iter().filter(|o| matches!(o.data, ObjectData::Mesh(x) if x == mi)).count() > 1 {
                            let name = unique_name(&scene.mesh_names[mi], |n| scene.mesh_names.iter().any(|x| x == n));
                            let m = scene.meshes[mi].clone();
                            let ni = scene.add_mesh(&name, m);
                            scene.objects[i].data = ObjectData::Mesh(ni);
                        }
                    }
                }
            }
            _ => return Err(format!("object.{name} is not implemented")),
        }
        Ok(true)
    }

    fn op_transform(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        let value = cmd.list("value")?.unwrap_or_default();
        let axis = match cmd.opt("orient_axis").map(|t| t.text.to_ascii_uppercase()).as_deref() {
            Some("X") => Vec3::new(1.0, 0.0, 0.0),
            Some("Y") => Vec3::new(0.0, 1.0, 0.0),
            _ => Vec3::new(0.0, 0.0, 1.0),
        };
        let constraint = cmd.opt("constraint_axis").map(|t| t.text);
        let mask = match constraint {
            Some(c) => {
                let b: Vec<bool> = c.split(',').map(|x| boolean(x).unwrap_or(true)).collect();
                Vec3::new(b.first().copied().unwrap_or(true) as u8 as f32, b.get(1).copied().unwrap_or(true) as u8 as f32, b.get(2).copied().unwrap_or(true) as u8 as f32)
            }
            None => Vec3::ONE,
        };
        let tvec = || -> Res<Vec3> {
            match value.as_slice() {
                [x, y, z] => Ok(Vec3::new(*x, *y, *z)),
                [s] => Ok(Vec3::splat(*s)),
                [] => Ok(Vec3::ZERO),
                v => Err(format!("value needs 3 numbers, got {}", v.len())),
            }
        };
        let angle = value.first().copied().unwrap_or(0.0);
        if self.bpy.mode == "EDIT" {
            let mi = self.edit_mesh(scene)?;
            let sel = self.bpy.sel.get(&mi).cloned().unwrap_or_default();
            let m = &mut scene.meshes[mi];
            let verts = sel.verts(m);
            if verts.is_empty() {
                return Ok(false);
            }
            let c = verts.iter().fold(Vec3::ZERO, |a, &v| a + m.positions[v as usize]) / verts.len() as f32;
            for &v in &verts {
                let p = &mut m.positions[v as usize];
                *p = match name {
                    "translate" => *p + tvec()?.mul_elem(mask),
                    "rotate" => c + Quat::from_axis_angle(axis, angle).rotate(*p - c),
                    "resize" => {
                        let s = tvec()?;
                        let s = Vec3::new(if mask.x > 0.0 { s.x } else { 1.0 }, if mask.y > 0.0 { s.y } else { 1.0 }, if mask.z > 0.0 { s.z } else { 1.0 });
                        c + (*p - c).mul_elem(s)
                    }
                    _ => {
                        let s = Vec3::new(if mask.x > 0.0 { -1.0 } else { 1.0 }, if mask.y > 0.0 { -1.0 } else { 1.0 }, if mask.z > 0.0 { -1.0 } else { 1.0 });
                        c + (*p - c).mul_elem(s)
                    }
                };
            }
            if name == "mirror" {
                let all: Vec<u32> = sel.faces(m);
                crate::ops::flip_normals(m, &all);
            }
            return Ok(true);
        }
        for i in Self::targets(scene) {
            let n = scene.objects[i].name.clone();
            let t = &mut scene.objects[i].transform;
            match name {
                "translate" => t.location += tvec()?.mul_elem(mask),
                "rotate" => {
                    t.rotation = (Quat::from_axis_angle(axis, angle) * t.rotation).normalize();
                    self.bpy.eulers.remove(&n);
                }
                "resize" => {
                    let s = tvec()?;
                    t.scale = t.scale.mul_elem(Vec3::new(if mask.x > 0.0 { s.x } else { 1.0 }, if mask.y > 0.0 { s.y } else { 1.0 }, if mask.z > 0.0 { s.z } else { 1.0 }));
                }
                _ => t.scale = t.scale.mul_elem(Vec3::new(if mask.x > 0.0 { -1.0 } else { 1.0 }, if mask.y > 0.0 { -1.0 } else { 1.0 }, if mask.z > 0.0 { -1.0 } else { 1.0 })),
            }
        }
        Ok(true)
    }

    fn op_uv(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        use crate::uv;
        let meshes: Vec<(usize, Vec<u32>)> = if let Ok(mi) = self.edit_mesh(scene) {
            let sel = self.bpy.sel.get(&mi).cloned().unwrap_or_default();
            let f = sel.faces(&scene.meshes[mi]);
            vec![(mi, f)]
        } else {
            Self::targets(scene)
                .into_iter()
                .filter_map(|i| match scene.objects[i].data {
                    ObjectData::Mesh(mi) => Some((mi, (0..scene.meshes[mi].faces.len() as u32).collect())),
                    _ => None,
                })
                .collect()
        };
        let angle = cmd.f("angle_limit")?.unwrap_or(66f32.to_radians());
        // Blender 4.x takes radians; older scripts pass degrees (above 2*pi).
        let angle = if angle > std::f32::consts::TAU { angle.to_radians() } else { angle };
        let margin = cmd.f("island_margin")?.or(cmd.f("margin")?).unwrap_or(0.0);
        for (mi, faces) in meshes {
            let m = &mut scene.meshes[mi];
            match name {
                "smart_project" => uv::smart_project(m, &faces, angle, margin, cmd.f("area_weight")?.unwrap_or(0.0)),
                "unwrap" => {
                    let _ = uv::unwrap(m, &faces, &uv::UnwrapOptions { margin: if margin > 0.0 { margin } else { 0.001 }, ..Default::default() });
                }
                "cube_project" => uv::cube_project(m, &faces, cmd.f("cube_size")?),
                "cylinder_project" => uv::cylinder_project(m, &faces, cmd.f("radius")?.unwrap_or(1.0)),
                "sphere_project" => uv::sphere_project(m, &faces),
                "reset" => uv::reset(m, &faces),
                "lightmap_pack" => uv::lightmap_pack(m, &faces, cmd.f("PREF_MARGIN_DIV")?.unwrap_or(0.1)),
                "project_from_view" => uv::project_front(m, &faces, cmd.b("scale_to_bounds")?.unwrap_or(false)),
                _ => return Err(format!("uv.{name} is not implemented")),
            }
        }
        Ok(true)
    }

    fn op_rig(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        let a = scene.active_object.ok_or("no active object")?;
        let oname = scene.objects[a].name.clone();
        match name {
            "transforms_clear" | "rot_clear" | "loc_clear" | "scale_clear" => {
                let sel = self.bpy.bone_sel.get(&oname).cloned().unwrap_or_default();
                for pb in scene.objects[a].pose.iter_mut().filter(|p| sel.is_empty() || sel.contains(&p.name)) {
                    if matches!(name, "transforms_clear" | "loc_clear") {
                        pb.location = Vec3::ZERO;
                    }
                    if matches!(name, "transforms_clear" | "rot_clear") {
                        pb.rotation_quaternion = Quat::IDENTITY;
                        pb.rotation_euler = Vec3::ZERO;
                        pb.rotation_axis_angle = [0.0, 0.0, 1.0, 0.0];
                    }
                    if matches!(name, "transforms_clear" | "scale_clear") {
                        pb.scale = Vec3::ONE;
                    }
                }
            }
            "select_all" => {
                let action = cmd.opt("action").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "TOGGLE".into());
                let all: BTreeSet<String> = scene.armature_of(a).map(|ar| ar.bones.iter().map(|b| b.name.clone()).collect()).unwrap_or_default();
                let cur = self.bpy.bone_sel.entry(oname).or_default();
                *cur = match action.as_str() {
                    "SELECT" => all,
                    "DESELECT" => BTreeSet::new(),
                    "INVERT" => all.difference(cur).cloned().collect(),
                    _ if cur.is_empty() => all,
                    _ => BTreeSet::new(),
                };
            }
            "bone_primitive_add" => {
                let ObjectData::Armature(ai) = scene.objects[a].data else { return Err("not an armature".into()) };
                let n = cmd.opt("name").map(|t| t.text).unwrap_or_else(|| "Bone".into());
                let head = scene.world_matrix(a).inverse().transform_point(Vec3::from_array(self.bpy.cursor));
                scene.armatures[ai].add_bone(Bone::new(&n, head, head + Vec3::new(0.0, 0.0, 1.0)));
            }
            _ => return Err(format!("{name} is not implemented")),
        }
        Ok(true)
    }

    fn op_anim(&mut self, scene: &mut Scene, cmd: &mut Cmd, name: &str) -> Res<bool> {
        let ty = cmd.opt("type").map(|t| t.text.to_ascii_uppercase()).unwrap_or_else(|| "LOCROTSCALE".into());
        let paths: &[&str] = match ty.as_str() {
            "LOCATION" | "LOC" => &["location"],
            "ROTATION" | "ROT" => &["rotation_euler"],
            "SCALING" | "SCALE" => &["scale"],
            "LOCROT" => &["location", "rotation_euler"],
            "LOCSCALE" => &["location", "scale"],
            "ROTSCALE" => &["rotation_euler", "scale"],
            _ => &["location", "rotation_euler", "scale"],
        };
        let frame = scene.frame_current as f32;
        for i in Self::targets(scene) {
            let o = &scene.objects[i];
            let n = o.name.clone();
            let order = self.bpy.euler_order(&n);
            let e = self.bpy.eulers.get(&n).copied().unwrap_or_else(|| o.transform.rotation.to_euler(order));
            let t = o.transform;
            for &p in paths {
                let vals = match p {
                    "location" => t.location.to_array(),
                    "scale" => t.scale.to_array(),
                    _ => e,
                };
                for (k, v) in vals.iter().enumerate() {
                    let mut sub = Cmd {
                        name: if name == "keyframe_delete" { "keyframe_delete".into() } else { "keyframe".into() },
                        pos: vec![
                            Token { key: None, text: n.clone(), quoted: true },
                            Token { key: None, text: p.into(), quoted: true },
                            Token { key: None, text: k.to_string(), quoted: false },
                            Token { key: None, text: fmt_f(frame), quoted: false },
                        ],
                        opts: Vec::new(),
                    };
                    if name != "keyframe_delete" {
                        sub.pos.push(Token { key: None, text: fmt_f(*v), quoted: false });
                    }
                    self.bpy_command(scene, &mut sub)?;
                }
            }
        }
        Ok(true)
    }

    fn op_render(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<bool> {
        let write = cmd.b("write_still")?.unwrap_or(false);
        let animation = cmd.b("animation")?.unwrap_or(false);
        let mut render_scene = scene.clone();
        for o in render_scene.objects.iter_mut() {
            if self.bpy.hide_render.contains(&o.name) {
                o.visible = false;
            }
        }
        let frames: Vec<i32> = if animation { (scene.frame_start..=scene.frame_end).collect() } else { vec![scene.frame_current] };
        let base = self.bpy.props.get("render.filepath").cloned().unwrap_or_else(|| "/tmp/".into());
        for f in frames {
            if animation {
                render_scene.frame_current = f;
                self.bpy.evaluate(&mut render_scene, f as f32);
            }
            let settings = self.bpy.render_settings(f);
            let r = crate::render::render(&render_scene, &settings);
            let mut img = r.color;
            img.name = "Render Result".into();
            match scene.images.iter_mut().find(|i| i.name == "Render Result") {
                Some(slot) => *slot = img.clone(),
                None => scene.images.push(img.clone()),
            }
            if write || animation {
                let path = if animation || base.ends_with('/') || base.is_empty() { format!("{base}{f:04}.png") } else if base.to_ascii_lowercase().ends_with(".png") { base.clone() } else { format!("{base}.png") };
                write_png(&img, &path)?;
                self.bpy.files_written.push(path);
            }
        }
        Ok(true)
    }

    fn op_file(&mut self, scene: &mut Scene, cmd: &mut Cmd, id: &str) -> Res<bool> {
        use crate::io::{self, Format};
        let path = cmd.opt("filepath").map(|t| t.text).unwrap_or_default();
        match id {
            "wm.read_homefile" | "wm.read_factory_settings" => {
                *scene = Scene::startup();
                self.bpy = State { registry: std::mem::take(&mut self.bpy.registry), replaced: true, ..State::default() };
                self.handles.clear();
                return Ok(true);
            }
            "wm.save_mainfile" | "wm.save_as_mainfile" => {
                if path.is_empty() {
                    return Err("save_mainfile needs filepath= (Polyamour scripts have no open file path)".into());
                }
                // A `.blend` name writes a .blend file, anything else polyops.
                let bytes = if path.to_ascii_lowercase().ends_with(".blend") {
                    let opts = io::blend::BlendExport { width: self.bpy.prop_f("render.resolution_x", 1920.0).max(1.0) as u32, height: self.bpy.prop_f("render.resolution_y", 1080.0).max(1.0) as u32 };
                    io::blend::export_blend(scene, &self.bpy.blend_actions(), opts)
                } else {
                    scene_to_polyops(scene).into_bytes()
                };
                std::fs::write(&path, bytes).map_err(|e| format!("cannot write {path}: {e}"))?;
                self.bpy.files_written.push(path);
                return Ok(true);
            }
            "wm.open_mainfile" => {
                let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
                let new = if path.ends_with(".poly") || bytes.starts_with(b"polyops ") {
                    let mut s = Scene::new();
                    run(&mut s, &String::from_utf8_lossy(&bytes)).map_err(|e| e.to_string())?;
                    s
                } else {
                    let f = Format::from_path(&path).or_else(|| Format::sniff(&bytes)).ok_or_else(|| format!("{path}: unknown file type"))?;
                    io::import(&bytes, f, &|_: &str| None).map_err(|e| e.to_string())?
                };
                *scene = new;
                self.bpy = State { registry: std::mem::take(&mut self.bpy.registry), replaced: true, ..State::default() };
                self.handles.clear();
                return Ok(true);
            }
            _ => {}
        }
        if path.is_empty() {
            return Err(format!("{id} needs filepath="));
        }
        let (import, fmt) = match id {
            "import_scene.gltf" => (true, None),
            "import_scene.obj" | "wm.obj_import" => (true, Some(Format::Obj)),
            "wm.stl_import" | "import_mesh.stl" => (true, Some(Format::Stl)),
            "wm.ply_import" | "import_mesh.ply" => (true, Some(Format::Ply)),
            "export_scene.gltf" => {
                let f = match cmd.opt("export_format").map(|t| t.text.to_ascii_uppercase()).as_deref() {
                    Some("GLTF_SEPARATE") | Some("GLTF_EMBEDDED") => Format::Gltf,
                    _ if path.to_ascii_lowercase().ends_with(".gltf") => Format::Gltf,
                    _ => Format::Glb,
                };
                (false, Some(f))
            }
            "export_scene.obj" | "wm.obj_export" => (false, Some(Format::Obj)),
            "wm.stl_export" | "export_mesh.stl" => (false, Some(Format::Stl)),
            "wm.ply_export" | "export_mesh.ply" => (false, Some(Format::Ply)),
            _ => return Err(format!("{id} is not implemented")),
        };
        if import {
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let f = fmt.or_else(|| Format::from_path(&path)).or_else(|| Format::sniff(&bytes)).ok_or_else(|| format!("{path}: unknown file type"))?;
            let dir = std::path::Path::new(&path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let resolve = move |name: &str| std::fs::read(dir.join(name)).ok();
            let added = io::import_into(scene, &bytes, f, &resolve).map_err(|e| e.to_string())?;
            for &i in &added {
                self.report.created.push(scene.objects[i].name.clone());
            }
            scene.selected = added.clone();
            scene.active_object = added.last().copied().or(scene.active_object);
        } else {
            let f = fmt.expect("export formats are known");
            let mut out = scene.clone();
            if cmd.b("use_selection")?.unwrap_or(false) {
                let keep: Vec<String> = scene.selected.iter().map(|&i| scene.objects[i].name.clone()).collect();
                let drop: Vec<usize> = (0..out.objects.len()).filter(|&i| !keep.contains(&out.objects[i].name)).collect();
                for &i in drop.iter().rev() {
                    out.remove_object(i);
                }
            }
            let bytes = io::export(&out, f).map_err(|e| e.to_string())?;
            std::fs::write(&path, bytes).map_err(|e| format!("cannot write {path}: {e}"))?;
            self.bpy.files_written.push(path);
        }
        Ok(true)
    }
}

/// Group name Blender gives new F-Curves.
fn default_group(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("pose.bones[\"") {
        if let Some((bone, _)) = rest.split_once("\"]") {
            return bone.to_string();
        }
    }
    match path {
        "location" | "rotation_euler" | "rotation_quaternion" | "rotation_axis_angle" | "scale" => "Object Transforms".into(),
        _ => String::new(),
    }
}

/// Write `img` as an 8-bit sRGB PNG.
pub fn write_png(img: &crate::scene::Image, path: &str) -> Res<()> {
    let enc = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    let mut data = Vec::with_capacity(img.pixels.len() * 4);
    for p in &img.pixels {
        data.extend([enc(p[0]), enc(p[1]), enc(p[2]), (p[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8]);
    }
    let file = std::fs::File::create(path).map_err(|e| format!("cannot write {path}: {e}"))?;
    let mut e = png::Encoder::new(std::io::BufWriter::new(file), img.width, img.height);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut w = e.write_header().map_err(|e| format!("png: {e}"))?;
    w.write_image_data(&data).map_err(|e| format!("png: {e}"))?;
    Ok(())
}
