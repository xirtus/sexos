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
}

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
        let i = self.scene.add_object(Object::new(name, data));
        self.select_only(Some(i));
        i
    }

    /// Run an operator by its Blender-style name. Unknown names are reported,
    /// not ignored, so missing features show up in the status bar.
    pub fn run(&mut self, op: &Op) {
        let arg = op.arg.as_str();
        match op.id.as_str() {
            "ed.undo" => match self.undo.undo(&mut self.scene) {
                Some(l) => self.report = format!("Undo {l}"),
                None => self.report = "Nothing to undo".into(),
            },
            "ed.redo" => match self.undo.redo(&mut self.scene) {
                Some(l) => self.report = format!("Redo {l}"),
                None => self.report = "Nothing to redo".into(),
            },
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
                    self.edit = EditSelection::default();
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
