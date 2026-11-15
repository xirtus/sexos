// SPDX-License-Identifier: GPL-3.0-only
//! The open document: the scene, undo history, interaction mode, current frame,
//! and the operators that change them. Editors read and change the document;
//! anything named like a Blender operator (`object.delete`) goes through
//! [`Doc::run`], so menus, hotkeys and scripts share one path.
use crate::ui::Op;
use sex_3d::modifiers::Modifier;
use sex_3d::scene::{Light, LightKind, Object, ObjectData, Scene};
use sex_3d::undo::UndoStack;
use sex_3d::{primitives, Vec3};
use std::collections::BTreeSet;

/// Blender's interaction modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Object,
    Edit,
    Sculpt,
    VertexPaint,
    WeightPaint,
    TexturePaint,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Object => "Object Mode",
            Mode::Edit => "Edit Mode",
            Mode::Sculpt => "Sculpt Mode",
            Mode::VertexPaint => "Vertex Paint",
            Mode::WeightPaint => "Weight Paint",
            Mode::TexturePaint => "Texture Paint",
        }
    }
    pub const ALL: [Mode; 6] = [Mode::Object, Mode::Edit, Mode::Sculpt, Mode::VertexPaint, Mode::WeightPaint, Mode::TexturePaint];
}

/// Edit-mode select mode (1, 2, 3 keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    Vertex,
    Edge,
    Face,
}

/// Edit-mode selection on the active mesh object.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditSelection {
    pub verts: BTreeSet<u32>,
    /// Edges as (low, high) vertex pairs.
    pub edges: BTreeSet<(u32, u32)>,
    pub faces: BTreeSet<u32>,
    /// The active element (drawn white), the last one clicked.
    pub active: Option<Elem>,
}

/// One edit-mode element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    Vert(u32),
    /// (low, high) vertex pair.
    Edge(u32, u32),
    Face(u32),
}

// ---- 3D Viewport tool settings (view3d agent) ----
/// The 3D Viewport's toolbar tools (Blender's object- and edit-mode tools).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    SelectBox,
    Cursor,
    Move,
    Rotate,
    Scale,
    Transform,
    Annotate,
    Measure,
    AddCube,
    Extrude,
    Inset,
    Bevel,
    LoopCut,
    Knife,
    PolyBuild,
    Spin,
    Smooth,
    EdgeSlide,
    ShrinkFatten,
    Shear,
    Rip,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::SelectBox => "Select Box",
            Tool::Cursor => "Cursor",
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            Tool::Scale => "Scale",
            Tool::Transform => "Transform",
            Tool::Annotate => "Annotate",
            Tool::Measure => "Measure",
            Tool::AddCube => "Add Cube",
            Tool::Extrude => "Extrude Region",
            Tool::Inset => "Inset Faces",
            Tool::Bevel => "Bevel",
            Tool::LoopCut => "Loop Cut",
            Tool::Knife => "Knife",
            Tool::PolyBuild => "Poly Build",
            Tool::Spin => "Spin",
            Tool::Smooth => "Smooth",
            Tool::EdgeSlide => "Edge Slide",
            Tool::ShrinkFatten => "Shrink/Fatten",
            Tool::Shear => "Shear",
            Tool::Rip => "Rip Region",
        }
    }
    pub const ALL: [Tool; 21] = [
        Tool::SelectBox,
        Tool::Cursor,
        Tool::Move,
        Tool::Rotate,
        Tool::Scale,
        Tool::Transform,
        Tool::Annotate,
        Tool::Measure,
        Tool::AddCube,
        Tool::Extrude,
        Tool::Inset,
        Tool::Bevel,
        Tool::LoopCut,
        Tool::Knife,
        Tool::PolyBuild,
        Tool::Spin,
        Tool::Smooth,
        Tool::EdgeSlide,
        Tool::ShrinkFatten,
        Tool::Shear,
        Tool::Rip,
    ];
    pub fn from_label(s: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.label() == s)
    }
}

/// How box selection combines with the selection (the Select Box tool's mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxMode {
    Set,
    Extend,
    Subtract,
    Invert,
    Intersect,
}
// ---- end of view3d tool settings ----

pub struct Doc {
    pub scene: Scene,
    pub undo: UndoStack,
    pub mode: Mode,
    pub select_mode: SelectMode,
    pub edit: EditSelection,
    pub frame: i32,
    pub playing: bool,
    /// File the document was opened from or saved to.
    pub path: Option<String>,
    pub dirty: bool,
    /// Last report line for the status bar ("Saved untitled.glb").
    pub report: String,
    /// Bumped whenever the scene's geometry or objects change (viewport caches).
    pub generation: u64,
    /// The last F12 render, linear RGBA, and its size.
    pub render: Option<sex_3d::scene::Image>,
    /// The workspace an operator asked to show (F12 shows "Rendering").
    pub want_workspace: Option<String>,
    /// Operators queued by editors this frame, run after drawing.
    pub queue: Vec<Op>,
    /// Text editor buffers: (name, text).
    pub texts: Vec<(String, String)>,

    // ---- 3D Viewport, transform and edit-mode state (view3d agent) ----
    /// The 3D cursor: where Shift+right-click puts it and new objects appear.
    pub cursor: Vec3,
    /// The 3D cursor's rotation (XYZ Euler, radians).
    pub cursor_rotation: Vec3,
    /// The active toolbar tool.
    pub tool: Tool,
    pub box_mode: BoxMode,
    pub pivot: crate::transform::Pivot,
    pub orientation: crate::transform::Orientation,
    /// Snapping toggle (the magnet; Shift+Tab). Snaps to increments.
    pub snap: bool,
    pub snap_target: crate::transform::SnapTarget,
    /// Proportional editing (O) and its radius and falloff.
    pub proportional: bool,
    pub proportional_size: f32,
    pub falloff: crate::transform::Falloff,
    /// Edit-mode hidden vertices (H); faces and edges touching them are hidden.
    pub edit_hidden: BTreeSet<u32>,
    /// A popup menu an editor's hotkey asked for (X's "Delete?", Shift+S...);
    /// the window opens it.
    pub menu: Option<crate::ui::Popup>,
    /// Operators that act on a 3D view (navigation, modal tools), queued for
    /// the viewport that asked for them.
    pub view_ops: Vec<Op>,
}

impl Default for Doc {
    fn default() -> Self {
        Doc::new(Scene::startup())
    }
}

impl Doc {
    pub fn new(mut scene: Scene) -> Doc {
        // Blender opens with the active object selected.
        if scene.selected.is_empty() {
            scene.selected.extend(scene.active_object);
        }
        Doc {
            scene,
            undo: UndoStack::new(),
            mode: Mode::Object,
            select_mode: SelectMode::Vertex,
            edit: EditSelection::default(),
            frame: 1,
            playing: false,
            path: None,
            dirty: false,
            report: String::new(),
            generation: 1,
            render: None,
            want_workspace: None,
            queue: Vec::new(),
            texts: Vec::new(),
            cursor: Vec3::ZERO,
            cursor_rotation: Vec3::ZERO,
            tool: Tool::SelectBox,
            box_mode: BoxMode::Set,
            pivot: crate::transform::Pivot::MedianPoint,
            orientation: crate::transform::Orientation::Global,
            snap: false,
            snap_target: crate::transform::SnapTarget::Increment,
            proportional: false,
            proportional_size: 1.0,
            falloff: crate::transform::Falloff::Smooth,
            edit_hidden: BTreeSet::new(),
            menu: None,
            view_ops: Vec::new(),
        }
    }

    /// Record an undo step named `label` before changing the scene.
    pub fn push_undo(&mut self, label: &str) {
        self.undo.push(label, &self.scene);
        self.dirty = true;
    }
    /// The scene changed: caches must rebuild.
    pub fn touch(&mut self) {
        self.generation += 1;
    }

    pub fn active(&self) -> Option<usize> {
        self.scene.active_object.filter(|&i| i < self.scene.objects.len())
    }
    pub fn is_selected(&self, i: usize) -> bool {
        self.scene.selected.contains(&i)
    }
    pub fn select_only(&mut self, i: Option<usize>) {
        self.scene.selected.clear();
        if let Some(i) = i {
            self.scene.selected.push(i);
            self.scene.active_object = Some(i);
        }
        self.touch();
    }
    pub fn toggle_select(&mut self, i: usize) {
        if let Some(p) = self.scene.selected.iter().position(|&s| s == i) {
            if self.scene.active_object == Some(i) {
                self.scene.selected.remove(p);
            } else {
                self.scene.active_object = Some(i);
            }
        } else {
            self.scene.selected.push(i);
            self.scene.active_object = Some(i);
        }
        self.touch();
    }

    /// Add an object at the 3D cursor (origin), selected and active, like Blender.
    pub fn add_object(&mut self, name: &str, data: ObjectData, label: &str) -> usize {
        self.push_undo(label);
        let mut o = Object::new(name, data);
        o.transform.location = self.cursor;
        let i = self.scene.add_object(o);
        self.select_only(Some(i));
        i
    }

    /// Run an operator by its Blender-style name. Unknown names are reported,
    /// not ignored, so missing features show up in the status bar.
    pub fn run(&mut self, op: &Op) {
        if self.run_view3d(op) || crate::edit::run(self, op) {
            return;
        }
        let arg = op.arg.as_str();
        match op.id.as_str() {
            "ed.undo" => {
                match self.undo.undo(&mut self.scene) {
                    Some(l) => self.report = format!("Undo {l}"),
                    None => self.report = "Nothing to undo".into(),
                }
                crate::edit::sanitize(self);
                self.touch();
            }
            "ed.redo" => {
                match self.undo.redo(&mut self.scene) {
                    Some(l) => self.report = format!("Redo {l}"),
                    None => self.report = "Nothing to redo".into(),
                }
                crate::edit::sanitize(self);
                self.touch();
            }
            "mesh.primitive_add" => {
                let (label, mesh) = match arg {
                    "plane" => ("Plane", primitives::plane(2.0)),
                    "cube" => ("Cube", primitives::cube(2.0)),
                    "circle" => ("Circle", circle(32, 1.0)),
                    "uv_sphere" => ("Sphere", primitives::uv_sphere(32, 16, 1.0)),
                    "ico_sphere" => ("Icosphere", primitives::ico_sphere(2, 1.0)),
                    "cylinder" => ("Cylinder", primitives::cylinder(32, 1.0, 2.0)),
                    "cone" => ("Cone", primitives::cone(32, 1.0, 0.0, 2.0)),
                    "torus" => ("Torus", primitives::torus(48, 12, 1.0, 0.25)),
                    "grid" => ("Grid", primitives::grid(10, 10, 2.0)),
                    _ => return self.unknown(op),
                };
                self.push_undo(&format!("Add {label}"));
                let i = self.scene.add_mesh_object(label, mesh);
                self.scene.objects[i].transform.location = self.cursor;
                if self.mode == Mode::Edit {
                    self.mode = Mode::Object;
                }
                self.select_only(Some(i));
            }
            "object.light_add" => {
                let kind = match arg {
                    "SUN" => LightKind::Sun,
                    "SPOT" => LightKind::Spot { angle: 45f32.to_radians(), blend: 0.15 },
                    "AREA" => LightKind::Area { size: 1.0 },
                    _ => LightKind::Point,
                };
                let name = match kind {
                    LightKind::Sun => "Sun",
                    LightKind::Spot { .. } => "Spot",
                    LightKind::Area { .. } => "Area",
                    LightKind::Point => "Point",
                };
                let power = if matches!(kind, LightKind::Sun) { 1.0 } else { 1000.0 };
                self.add_object(name, ObjectData::Light(Light { kind, power, ..Default::default() }), &format!("Add {name}"));
            }
            "object.camera_add" => {
                self.add_object("Camera", ObjectData::Camera(Default::default()), "Add Camera");
            }
            "object.empty_add" => {
                self.add_object("Empty", ObjectData::Empty, "Add Empty");
            }
            "object.select_all" => {
                let all: Vec<usize> = (0..self.scene.objects.len()).collect();
                match arg {
                    "DESELECT" => self.scene.selected.clear(),
                    "INVERT" => self.scene.selected = all.into_iter().filter(|i| !self.scene.selected.contains(i)).collect(),
                    _ => self.scene.selected = all,
                }
                self.touch();
            }
            "object.delete" => {
                let mut sel = self.scene.selected.clone();
                if sel.is_empty() {
                    return;
                }
                self.push_undo("Delete");
                sel.sort_unstable();
                for &i in sel.iter().rev() {
                    self.scene.remove_object(i);
                }
                self.report = format!("Deleted {} object(s)", sel.len());
                self.touch();
            }
            "object.duplicate" => {
                let sel = self.scene.selected.clone();
                if sel.is_empty() {
                    return;
                }
                self.push_undo("Duplicate Objects");
                let mut new = Vec::new();
                for i in sel {
                    let mut o = self.scene.objects[i].clone();
                    if let ObjectData::Mesh(mi) = o.data {
                        let name = self.scene.mesh_names[mi].clone();
                        let m = self.scene.meshes[mi].clone();
                        o.data = ObjectData::Mesh(self.scene.add_mesh(&name, m));
                    }
                    new.push(self.scene.add_object(o));
                }
                self.scene.selected = new.clone();
                self.scene.active_object = new.last().copied();
                self.touch();
            }
            "object.hide_view_set" => {
                let sel = self.scene.selected.clone();
                for i in sel {
                    self.scene.objects[i].visible = false;
                }
                self.scene.selected.clear();
                self.touch();
            }
            "object.hide_view_clear" => {
                for o in &mut self.scene.objects {
                    o.visible = true;
                }
                self.touch();
            }
            "object.shade_smooth" | "object.shade_flat" => {
                let smooth = op.id == "object.shade_smooth";
                self.push_undo(if smooth { "Shade Smooth" } else { "Shade Flat" });
                for i in self.scene.selected.clone() {
                    if let ObjectData::Mesh(mi) = self.scene.objects[i].data {
                        for f in &mut self.scene.meshes[mi].faces {
                            f.smooth = smooth;
                        }
                    }
                }
                self.touch();
            }
            "object.modifier_add" => {
                let Some(a) = self.active() else { return };
                let m = match arg {
                    "SUBSURF" => Modifier::Subdivision { levels: 1, simple: false },
                    "MIRROR" => Modifier::Mirror { x: true, y: false, z: false, merge: true, merge_distance: 0.001 },
                    "ARRAY" => Modifier::Array { count: 2, relative: [1.0, 0.0, 0.0], constant: [0.0; 3], merge: false },
                    "SOLIDIFY" => Modifier::Solidify { thickness: 0.01, offset: -1.0 },
                    "BEVEL" => Modifier::Bevel { width: 0.1, segments: 1, angle: 30f32.to_radians() },
                    "TRIANGULATE" => Modifier::Triangulate,
                    "WELD" => Modifier::Weld { distance: 0.001 },
                    "DISPLACE" => Modifier::Displace { strength: 1.0 },
                    "SMOOTH" => Modifier::Smooth { factor: 0.5, iterations: 1 },
                    _ => return self.unknown(op),
                };
                if !matches!(self.scene.objects[a].data, ObjectData::Mesh(_)) {
                    self.report = "Modifiers need a mesh object".into();
                    return;
                }
                self.push_undo("Add Modifier");
                self.scene.objects[a].modifiers.push(m);
                self.touch();
            }
            "object.mode_set" => {
                let want = match arg {
                    "EDIT" => Mode::Edit,
                    "SCULPT" => Mode::Sculpt,
                    "VERTEX_PAINT" => Mode::VertexPaint,
                    "WEIGHT_PAINT" => Mode::WeightPaint,
                    "TEXTURE_PAINT" => Mode::TexturePaint,
                    "TOGGLE_EDIT" => {
                        if self.mode == Mode::Edit {
                            Mode::Object
                        } else {
                            Mode::Edit
                        }
                    }
                    _ => Mode::Object,
                };
                if want != Mode::Object && !self.active().is_some_and(|a| matches!(self.scene.objects[a].data, ObjectData::Mesh(_))) {
                    self.report = "Active object is not a mesh".into();
                    return;
                }
                if want == Mode::Edit && self.mode != Mode::Edit {
                    // A fresh mesh in Blender enters Edit Mode all selected.
                    self.edit = EditSelection::default();
                    self.edit_hidden.clear();
                    self.mode = want;
                    crate::edit::select_all(self, "SELECT");
                }
                self.mode = want;
                self.touch();
            }
            "object.location_clear" | "object.rotation_clear" | "object.scale_clear" => {
                self.push_undo("Clear Transform");
                for i in self.scene.selected.clone() {
                    let t = &mut self.scene.objects[i].transform;
                    match op.id.as_str() {
                        "object.location_clear" => t.location = Vec3::ZERO,
                        "object.rotation_clear" => t.rotation = Default::default(),
                        _ => t.scale = Vec3::ONE,
                    }
                }
                self.touch();
            }
            "screen.frame_jump" => {
                self.frame = if arg == "END" { self.scene.frame_end } else { self.scene.frame_start };
            }
            "screen.frame_offset" => {
                self.frame += arg.parse::<i32>().unwrap_or(1);
            }
            "screen.animation_play" => self.playing = !self.playing,
            "screen.workspace" => self.want_workspace = Some(arg.to_string()),
            _ => self.unknown(op),
        }
    }

    fn unknown(&mut self, op: &Op) {
        self.report = format!("{} is not available yet", op.id);
    }
}

// ---- 3D Viewport operators (view3d agent) ----

/// Operators that act on a 3D view (its camera, the pointer, a modal tool).
/// [`Doc::run`] queues them on [`Doc::view_ops`] for the viewport, which runs
/// them on its next frame. A menu tags them with "@x,y;" (a point in the area
/// it was opened from) so the right viewport takes them.
pub fn is_view_op(id: &str) -> bool {
    matches!(
        id,
        "view3d.view_all"
            | "view3d.view_selected"
            | "view3d.view_axis"
            | "view3d.view_camera"
            | "view3d.view_persportho"
            | "view3d.view_orbit"
            | "view3d.view_pan"
            | "view3d.zoom"
            | "view3d.toggle_xray"
            | "view3d.toolbar"
            | "view3d.sidebar"
            | "view3d.camera_to_view"
            | "view3d.object_as_camera"
            | "view3d.view_center_cursor"
            | "view3d.select_box"
            | "view3d.shading"
            | "view3d.overlay"
            | "transform.translate"
            | "transform.rotate"
            | "transform.resize"
            | "object.duplicate_move"
            | "object.duplicate_move_linked"
            | "mesh.duplicate_move"
            | "mesh.extrude_region_move"
            | "mesh.extrude_faces_indiv_move"
            | "mesh.inset"
            | "mesh.bevel"
            | "mesh.loopcut_slide"
            | "mesh.knife_tool"
            | "object.move_to_collection_menu"
    )
}

/// Strip a viewport tag ("@x,y;") from an operator argument: (point, rest).
pub fn split_view_tag(arg: &str) -> (Option<(i32, i32)>, &str) {
    if let Some(rest) = arg.strip_prefix('@') {
        if let Some((pt, tail)) = rest.split_once(';') {
            if let Some((x, y)) = pt.split_once(',') {
                if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                    return (Some((x, y)), tail);
                }
            }
        }
    }
    (None, arg)
}

impl Doc {
    /// Mean world position of what is selected: objects' origins in Object
    /// Mode, selected vertices in Edit Mode.
    pub fn selection_center(&self) -> Option<Vec3> {
        let pts: Vec<Vec3> = if self.mode == Mode::Edit {
            let (oi, mi) = crate::edit::active_mesh(self)?;
            let w = self.scene.world_matrix(oi);
            crate::edit::selected_verts(self).iter().filter_map(|&v| self.scene.meshes[mi].positions.get(v as usize)).map(|&p| w.transform_point(p)).collect()
        } else {
            self.scene.selected.iter().filter(|&&i| i < self.scene.objects.len()).map(|&i| self.world_location(i)).collect()
        };
        if pts.is_empty() {
            return None;
        }
        Some(pts.iter().fold(Vec3::ZERO, |a, &b| a + b) / pts.len() as f32)
    }
    pub fn world_location(&self, i: usize) -> Vec3 {
        self.scene.world_matrix(i).transform_point(Vec3::ZERO)
    }
    /// Move object `i` so its origin is at world point `p`.
    pub fn set_world_location(&mut self, i: usize, p: Vec3) {
        let local = match self.scene.objects[i].parent {
            Some(pi) => self.scene.world_matrix(pi).inverse().transform_point(p),
            None => p,
        };
        self.scene.objects[i].transform.location = local;
    }

    fn context_set(&mut self, arg: &str) {
        use crate::transform::{Falloff, Orientation, Pivot, SnapTarget};
        let (key, val) = arg.split_once('=').unwrap_or((arg, ""));
        match key {
            "pivot" => self.pivot = Pivot::from_id(val).unwrap_or(self.pivot),
            "orientation" => self.orientation = Orientation::from_id(val).unwrap_or(self.orientation),
            "snap" => self.snap = if val == "toggle" { !self.snap } else { val == "true" },
            "snap_target" => self.snap_target = SnapTarget::from_id(val).unwrap_or(self.snap_target),
            "proportional" => self.proportional = if val == "toggle" { !self.proportional } else { val == "true" },
            "falloff" => self.falloff = Falloff::from_id(val).unwrap_or(self.falloff),
            "tool" => {
                if let Some(t) = Tool::from_label(val) {
                    self.tool = t;
                }
            }
            "box_mode" => {
                self.box_mode = match val {
                    "EXTEND" => BoxMode::Extend,
                    "SUB" => BoxMode::Subtract,
                    "XOR" => BoxMode::Invert,
                    "AND" => BoxMode::Intersect,
                    _ => BoxMode::Set,
                }
            }
            _ => self.report = format!("Unknown setting {key}"),
        }
    }

    fn mesh_of(&self, i: usize) -> Option<usize> {
        match self.scene.objects.get(i)?.data {
            ObjectData::Mesh(mi) => Some(mi),
            _ => None,
        }
    }

    /// The 3D Viewport's object operators. Returns false for ids it does not know.
    fn run_view3d(&mut self, op: &Op) -> bool {
        use sex_3d::scene::Transform;
        use sex_3d::Mat4;
        let arg = op.arg.as_str();
        if is_view_op(&op.id) {
            self.view_ops.push(op.clone());
            return true;
        }
        match op.id.as_str() {
            "wm.redraw" => {}
            "wm.context_set" => self.context_set(arg),
            "object.select_all" if arg == "TOGGLE" => {
                let arg = if self.scene.selected.is_empty() { "SELECT" } else { "DESELECT" };
                self.run(&Op::with("object.select_all", arg));
            }
            "object.select_by_type" => {
                let want = |d: &ObjectData| match arg {
                    "MESH" => matches!(d, ObjectData::Mesh(_)),
                    "CAMERA" => matches!(d, ObjectData::Camera(_)),
                    "LIGHT" => matches!(d, ObjectData::Light(_)),
                    _ => matches!(d, ObjectData::Empty),
                };
                for i in 0..self.scene.objects.len() {
                    if want(&self.scene.objects[i].data) && self.scene.objects[i].visible && !self.is_selected(i) {
                        self.scene.selected.push(i);
                    }
                }
                self.touch();
            }
            "object.select_camera" => {
                if let Some(c) = self.scene.active_camera {
                    self.select_only(Some(c));
                }
            }
            "object.hide_view_set" if arg == "UNSELECTED" => {
                for i in 0..self.scene.objects.len() {
                    if !self.is_selected(i) {
                        self.scene.objects[i].visible = false;
                    }
                }
                self.touch();
            }
            "object.duplicate_linked" => {
                let sel = self.scene.selected.clone();
                if sel.is_empty() {
                    return true;
                }
                self.push_undo("Duplicate Linked");
                let new: Vec<usize> = sel.iter().map(|&i| self.scene.add_object(self.scene.objects[i].clone())).collect();
                self.scene.selected = new.clone();
                self.scene.active_object = new.last().copied();
                self.touch();
            }
            "object.transform_apply" => {
                self.push_undo("Apply Object Transform");
                let (loc, rot, scl) = match arg {
                    "LOCATION" => (true, false, false),
                    "ROTATION" => (false, true, false),
                    "SCALE" => (false, false, true),
                    "ROTSCALE" => (false, true, true),
                    _ => (true, true, true),
                };
                let mut done = 0;
                for i in self.scene.selected.clone() {
                    let Some(mi) = self.mesh_of(i) else { continue };
                    let t = self.scene.objects[i].transform;
                    let keep = Transform {
                        location: if loc { Vec3::ZERO } else { t.location },
                        rotation: if rot { Default::default() } else { t.rotation },
                        scale: if scl { Vec3::ONE } else { t.scale },
                    };
                    let m: Mat4 = keep.matrix().inverse() * t.matrix();
                    self.scene.meshes[mi].transform(&m);
                    self.scene.objects[i].transform = keep;
                    done += 1;
                }
                self.report = format!("Applied transform to {done} object(s)");
                self.touch();
            }
            "object.convert" => {
                self.push_undo("Convert to Mesh");
                for i in self.scene.selected.clone() {
                    let Some(mi) = self.mesh_of(i) else { continue };
                    if let Some(m) = self.scene.evaluated_mesh(i) {
                        self.scene.meshes[mi] = m;
                        self.scene.objects[i].modifiers.clear();
                    }
                }
                self.touch();
            }
            "object.origin_set" => {
                self.push_undo("Set Origin");
                for i in self.scene.selected.clone() {
                    let Some(mi) = self.mesh_of(i) else { continue };
                    let b = self.scene.meshes[mi].bounds();
                    if b.is_empty() {
                        continue;
                    }
                    let c = match arg {
                        "ORIGIN_CURSOR" => self.scene.world_matrix(i).inverse().transform_point(self.cursor),
                        _ => b.center(),
                    };
                    self.scene.meshes[mi].translate(-c);
                    if arg != "GEOMETRY_ORIGIN" {
                        let t = self.scene.objects[i].transform;
                        self.scene.objects[i].transform.location = t.location + t.matrix().transform_vector(c);
                    }
                }
                self.touch();
            }
            "object.parent_set" => {
                let Some(a) = self.active() else { return true };
                self.push_undo("Make Parent");
                let pw = self.scene.world_matrix(a);
                for i in self.scene.selected.clone() {
                    if i == a {
                        continue;
                    }
                    // Refuse loops: the new parent must not be below the child.
                    let mut p = Some(a);
                    let mut looped = false;
                    while let Some(pi) = p {
                        if pi == i {
                            looped = true;
                            break;
                        }
                        p = self.scene.objects[pi].parent;
                    }
                    if looped {
                        self.report = "Loop in parents".into();
                        continue;
                    }
                    let local = pw.inverse() * self.scene.world_matrix(i);
                    self.scene.objects[i].parent = Some(a);
                    self.scene.objects[i].transform = Transform::from_matrix(&local);
                }
                self.touch();
            }
            "object.parent_clear" => {
                self.push_undo("Clear Parent");
                for i in self.scene.selected.clone() {
                    if arg == "CLEAR_KEEP_TRANSFORM" {
                        let w = self.scene.world_matrix(i);
                        self.scene.objects[i].transform = Transform::from_matrix(&w);
                    }
                    self.scene.objects[i].parent = None;
                }
                self.touch();
            }
            "object.join" => self.join(),
            "object.move_to_collection" => {
                if self.scene.selected.is_empty() {
                    return true;
                }
                self.push_undo("Move to Collection");
                let c = if arg == "NEW" { self.scene.add_collection("Collection", None) } else { arg.parse::<usize>().unwrap_or(0).min(self.scene.collections.len().saturating_sub(1)) };
                for i in self.scene.selected.clone() {
                    self.scene.objects[i].collection = c;
                }
                let name = self.scene.collections.get(c).map(|c| c.name.clone()).unwrap_or_default();
                self.report = format!("Moved to {name}");
                self.touch();
            }
            "view3d.snap_cursor_to_selected" => {
                if let Some(c) = self.selection_center() {
                    self.cursor = c;
                }
            }
            "view3d.snap_cursor_to_center" => {
                self.cursor = Vec3::ZERO;
                self.cursor_rotation = Vec3::ZERO;
            }
            "view3d.snap_cursor_to_active" => {
                if let Some(a) = self.active() {
                    self.cursor = self.world_location(a);
                }
            }
            "view3d.snap_cursor_to_grid" => self.cursor = round_grid(self.cursor),
            "view3d.snap_selected_to_cursor" | "view3d.snap_selected_to_active" | "view3d.snap_selected_to_grid" => {
                let goal = match op.id.as_str() {
                    "view3d.snap_selected_to_active" => match self.active() {
                        Some(a) => Some(self.world_location(a)),
                        None => return true,
                    },
                    "view3d.snap_selected_to_cursor" => Some(self.cursor),
                    _ => None,
                };
                self.push_undo("Snap Selection");
                let center = self.selection_center().unwrap_or(Vec3::ZERO);
                let to = |p: Vec3| match goal {
                    Some(g) if arg == "OFFSET" => p + (g - center),
                    Some(g) => g,
                    None => round_grid(p),
                };
                if self.mode == Mode::Edit {
                    if let Some((oi, mi)) = crate::edit::active_mesh(self) {
                        let w = self.scene.world_matrix(oi);
                        let inv = w.inverse();
                        for v in crate::edit::selected_verts(self) {
                            let p = w.transform_point(self.scene.meshes[mi].positions[v as usize]);
                            self.scene.meshes[mi].positions[v as usize] = inv.transform_point(to(p));
                        }
                    }
                } else {
                    for i in self.scene.selected.clone() {
                        if Some(i) == self.active() && op.id == "view3d.snap_selected_to_active" {
                            continue;
                        }
                        let p = self.world_location(i);
                        self.set_world_location(i, to(p));
                    }
                }
                self.touch();
            }
            _ => return false,
        }
        true
    }

    /// Ctrl+J: join the selected meshes into the active one.
    fn join(&mut self) {
        let Some(a) = self.active() else { return };
        let Some(ma) = self.mesh_of(a) else {
            self.report = "Active object is not a mesh".into();
            return;
        };
        let others: Vec<usize> = self.scene.selected.iter().copied().filter(|&i| i != a && self.mesh_of(i).is_some()).collect();
        if others.is_empty() {
            self.report = "No other selected meshes to join".into();
            return;
        }
        self.push_undo("Join");
        let inv = self.scene.world_matrix(a).inverse();
        for &i in &others {
            let mi = self.mesh_of(i).unwrap_or(0);
            let mut m = self.scene.meshes[mi].clone();
            m.transform(&(inv * self.scene.world_matrix(i)));
            // Bring the other object's material slots along.
            let slots = self.scene.objects[i].materials.clone();
            for f in &mut m.faces {
                f.material = match slots.get(f.material as usize).copied() {
                    Some(mat) => {
                        let mine = &mut self.scene.objects[a].materials;
                        let k = mine.iter().position(|&x| x == mat).unwrap_or_else(|| {
                            mine.push(mat);
                            mine.len() - 1
                        });
                        k as u16
                    }
                    None => 0,
                };
            }
            self.scene.meshes[ma].append(&m);
        }
        let mut others = others;
        others.sort_unstable();
        for &i in others.iter().rev() {
            self.scene.remove_object(i);
        }
        let a = self.scene.active_object;
        self.select_only(a);
        self.report = "Joined".into();
    }
}

/// Round a point to the 1 m grid.
pub fn round_grid(p: Vec3) -> Vec3 {
    Vec3::new(p.x.round(), p.y.round(), p.z.round())
}
// ---- end of 3D Viewport operators ----

/// A circle of `n` vertices (Blender adds it as an n-gon-less ring; we fill it).
fn circle(n: u32, r: f32) -> sex_3d::Mesh {
    let mut m = sex_3d::Mesh::new();
    for i in 0..n {
        let a = core::f32::consts::TAU * i as f32 / n as f32;
        m.add_vertex(Vec3::new(r * a.cos(), r * a.sin(), 0.0));
    }
    m.add_face(&(0..n).collect::<Vec<_>>());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_add_delete_undo() {
        let mut d = Doc::default();
        let n = d.scene.objects.len();
        d.run(&Op::with("mesh.primitive_add", "uv_sphere"));
        assert_eq!(d.scene.objects.len(), n + 1);
        assert_eq!(d.scene.objects[d.active().unwrap()].name, "Sphere");
        d.run(&Op::new("object.delete"));
        assert_eq!(d.scene.objects.len(), n);
        d.run(&Op::new("ed.undo"));
        assert_eq!(d.scene.objects.len(), n + 1);
        d.run(&Op::with("object.modifier_add", "SUBSURF"));
        assert_eq!(d.scene.objects[d.active().unwrap()].modifiers.len(), 1);
        d.run(&Op::new("no.such_op"));
        assert!(d.report.contains("not available"));
    }
}
