// SPDX-License-Identifier: GPL-3.0-only
//! Blender's modal transforms: G (Move), R (Rotate) and S (Scale) on the
//! selected objects or, in Edit Mode, the selected vertices.
//!
//! The pointer drives the value; X/Y/Z constrain to an axis (twice: the local
//! axis, a third time: free), Shift+X/Y/Z to the plane through the other two;
//! typing a number ("2", "-1.5", "90") sets it exactly; snapping (the magnet,
//! Shift+Tab during the transform) snaps to increments; Enter or a left click
//! confirms, Esc or a right click cancels. Proportional editing (O) moves
//! nearby unselected vertices too, with the falloff and radius chosen in the
//! header (PageUp/PageDown or the wheel change the radius while moving).
use crate::doc::{Doc, Elem, Mode};
use crate::editors::view3d::proj::Proj;
use crate::input::{Key, Mods};
use sex_3d::scene::Transform as Trs;
use sex_3d::{Mat4, Quat, Vec3};

/// Transform orientations (the header's "Global" dropdown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    Global,
    Local,
    Normal,
    Gimbal,
    View,
    Cursor,
    Parent,
}

impl Orientation {
    pub const ALL: [Orientation; 7] = [Orientation::Global, Orientation::Local, Orientation::Normal, Orientation::Gimbal, Orientation::View, Orientation::Cursor, Orientation::Parent];
    pub fn label(self) -> &'static str {
        match self {
            Orientation::Global => "Global",
            Orientation::Local => "Local",
            Orientation::Normal => "Normal",
            Orientation::Gimbal => "Gimbal",
            Orientation::View => "View",
            Orientation::Cursor => "Cursor",
            Orientation::Parent => "Parent",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Orientation::Global => "GLOBAL",
            Orientation::Local => "LOCAL",
            Orientation::Normal => "NORMAL",
            Orientation::Gimbal => "GIMBAL",
            Orientation::View => "VIEW",
            Orientation::Cursor => "CURSOR",
            Orientation::Parent => "PARENT",
        }
    }
    pub fn from_id(s: &str) -> Option<Orientation> {
        Orientation::ALL.into_iter().find(|o| o.id() == s)
    }
}

/// Transform pivot points (the header's pivot dropdown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pivot {
    BoundingBoxCenter,
    Cursor,
    IndividualOrigins,
    MedianPoint,
    ActiveElement,
}

impl Pivot {
    pub const ALL: [Pivot; 5] = [Pivot::BoundingBoxCenter, Pivot::Cursor, Pivot::IndividualOrigins, Pivot::MedianPoint, Pivot::ActiveElement];
    pub fn label(self) -> &'static str {
        match self {
            Pivot::BoundingBoxCenter => "Bounding Box Center",
            Pivot::Cursor => "3D Cursor",
            Pivot::IndividualOrigins => "Individual Origins",
            Pivot::MedianPoint => "Median Point",
            Pivot::ActiveElement => "Active Element",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Pivot::BoundingBoxCenter => "BOUNDING_BOX_CENTER",
            Pivot::Cursor => "CURSOR",
            Pivot::IndividualOrigins => "INDIVIDUAL_ORIGINS",
            Pivot::MedianPoint => "MEDIAN_POINT",
            Pivot::ActiveElement => "ACTIVE_ELEMENT",
        }
    }
    pub fn from_id(s: &str) -> Option<Pivot> {
        Pivot::ALL.into_iter().find(|o| o.id() == s)
    }
}

/// What snapping snaps to. Only increments are implemented; the others are
/// listed as Blender lists them and fall back to increments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapTarget {
    Increment,
    Vertex,
    Edge,
    Face,
    Volume,
}

impl SnapTarget {
    pub const ALL: [SnapTarget; 5] = [SnapTarget::Increment, SnapTarget::Vertex, SnapTarget::Edge, SnapTarget::Face, SnapTarget::Volume];
    pub fn label(self) -> &'static str {
        match self {
            SnapTarget::Increment => "Increment",
            SnapTarget::Vertex => "Vertex",
            SnapTarget::Edge => "Edge",
            SnapTarget::Face => "Face Project",
            SnapTarget::Volume => "Volume",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            SnapTarget::Increment => "INCREMENT",
            SnapTarget::Vertex => "VERTEX",
            SnapTarget::Edge => "EDGE",
            SnapTarget::Face => "FACE",
            SnapTarget::Volume => "VOLUME",
        }
    }
    pub fn from_id(s: &str) -> Option<SnapTarget> {
        SnapTarget::ALL.into_iter().find(|o| o.id() == s)
    }
}

/// Proportional editing falloff curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    Smooth,
    Sphere,
    Root,
    InverseSquare,
    Sharp,
    Linear,
    Constant,
}

impl Falloff {
    pub const ALL: [Falloff; 7] = [Falloff::Smooth, Falloff::Sphere, Falloff::Root, Falloff::InverseSquare, Falloff::Sharp, Falloff::Linear, Falloff::Constant];
    pub fn label(self) -> &'static str {
        match self {
            Falloff::Smooth => "Smooth",
            Falloff::Sphere => "Sphere",
            Falloff::Root => "Root",
            Falloff::InverseSquare => "Inverse Square",
            Falloff::Sharp => "Sharp",
            Falloff::Linear => "Linear",
            Falloff::Constant => "Constant",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Falloff::Smooth => "SMOOTH",
            Falloff::Sphere => "SPHERE",
            Falloff::Root => "ROOT",
            Falloff::InverseSquare => "INVERSE_SQUARE",
            Falloff::Sharp => "SHARP",
            Falloff::Linear => "LINEAR",
            Falloff::Constant => "CONSTANT",
        }
    }
    pub fn from_id(s: &str) -> Option<Falloff> {
        Falloff::ALL.into_iter().find(|o| o.id() == s)
    }
    /// Weight for `t` = 1 at the selection, 0 at the radius (Blender's curves).
    pub fn weight(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Falloff::Smooth => 3.0 * t * t - 2.0 * t * t * t,
            Falloff::Sphere => (2.0 * t - t * t).sqrt(),
            Falloff::Root => t.sqrt(),
            Falloff::InverseSquare => t * (2.0 - t),
            Falloff::Sharp => t * t,
            Falloff::Linear => t,
            Falloff::Constant => 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TMode {
    Translate,
    Rotate,
    Resize,
}

impl TMode {
    pub fn undo_label(self) -> &'static str {
        match self {
            TMode::Translate => "Move",
            TMode::Rotate => "Rotate",
            TMode::Resize => "Resize",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Constraint {
    None,
    /// Along one axis of the constraint orientation.
    Axis(usize),
    /// In the plane normal to one axis.
    Plane(usize),
    /// Along a world direction (extrude along the normal).
    Dir(Vec3),
}

/// The transform as computed from the pointer or typed value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Xf {
    Move(Vec3),
    Rot(Vec3, f32),
    /// Scale factors along the orientation's three axes.
    Scale([f32; 3]),
}

#[derive(Clone, Debug)]
struct ObjOrig {
    i: usize,
    t: Trs,
    world_loc: Vec3,
    parent_inv: Option<Mat4>,
}

#[derive(Clone, Debug)]
struct VertOrig {
    mesh: usize,
    w: Mat4,
    inv: Mat4,
    /// All the mesh's positions before the transform (object space).
    orig: Vec<Vec3>,
    /// Selected vertices.
    sel: Vec<u32>,
    /// Vertices that move and their proportional weight (1 for selected).
    moved: Vec<(u32, f32)>,
}

#[derive(Clone, Debug)]
enum Targets {
    Objects(Vec<ObjOrig>),
    Verts(VertOrig),
    /// Bones (Edit and Pose Mode) and lattice points (rig.rs).
    Rig(crate::rig::xform::Targets),
}

/// What a key did to a running transform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyResult {
    Continue,
    Confirm,
    Cancel,
    Ignored,
}

/// A running G/R/S transform.
#[derive(Clone, Debug)]
pub struct Modal {
    pub mode: TMode,
    pub constraint: Constraint,
    /// Orientation the constraint axes are in.
    pub space: Orientation,
    /// World directions of that orientation's X, Y, Z.
    pub basis: [Vec3; 3],
    /// Typed value ("" = use the pointer).
    pub numeric: String,
    /// Shift+Tab inverts the snapping toggle while transforming.
    pub snap_invert: bool,
    pub start: (f32, f32),
    pub mouse: (f32, f32),
    /// The pivot in world space.
    pub center: Vec3,
    angle_acc: f32,
    last_angle: f32,
    targets: Targets,
    pivot: crate::transform::Pivot,
    /// Started by dragging a gizmo handle: releasing the button confirms.
    pub release_confirms: bool,
    /// The operator that started this move (duplicate, extrude) already
    /// recorded the undo step: confirming adds none, so one Ctrl+Z undoes both.
    pub undo_recorded: bool,
    /// The last computed transform.
    pub xf: Xf,
    /// Header text ("D: 2.0000 m (2, 0, 0) along global X").
    pub header: String,
    /// Ctrl+R in armature Edit Mode: the rotation rolls the bones.
    pub roll: bool,
}

fn basis_of(m: &Mat4) -> [Vec3; 3] {
    [Vec3::X, Vec3::Y, Vec3::Z].map(|a| {
        let v = m.transform_vector(a).normalize();
        if v == Vec3::ZERO {
            a
        } else {
            v
        }
    })
}

/// World axes of an orientation for the current selection.
pub fn orientation_basis(d: &Doc, o: Orientation, proj: &Proj) -> [Vec3; 3] {
    match o {
        Orientation::Global => [Vec3::X, Vec3::Y, Vec3::Z],
        Orientation::View => [proj.right, proj.up, proj.back],
        Orientation::Cursor => {
            let q = Quat::from_euler_xyz(d.cursor_rotation.x, d.cursor_rotation.y, d.cursor_rotation.z);
            [q.rotate(Vec3::X), q.rotate(Vec3::Y), q.rotate(Vec3::Z)]
        }
        Orientation::Normal if d.mode == Mode::Edit => {
            let n = crate::edit::selection_normal(d);
            match n {
                Some(z) => {
                    let x = z.any_orthonormal();
                    let y = z.cross(x).normalize();
                    [x, y, z]
                }
                None => orientation_basis(d, Orientation::Local, proj),
            }
        }
        _ => match d.active() {
            Some(a) => basis_of(&d.scene.world_matrix(a)),
            None => [Vec3::X, Vec3::Y, Vec3::Z],
        },
    }
}

fn parse_numeric(s: &str) -> Option<f32> {
    if s.is_empty() || s == "-" {
        return None;
    }
    crate::ui::eval_number(s)
}

/// Format a number like Blender's transform header (trailing zeros dropped).
fn short(v: f32) -> String {
    crate::ui::trim_number(v)
}

impl Modal {
    /// Start a transform of the selection, or None when nothing is selected.
    pub fn start(d: &Doc, mode: TMode, proj: &Proj, mouse: (f32, f32)) -> Option<Modal> {
        let pivot = d.pivot;
        let rig = d.mode == Mode::Pose || crate::rig::edit_armature(d).is_some() || crate::rig::edit_lattice(d).is_some();
        let (targets, points) = if rig {
            let (t, pts) = crate::rig::xform::start(d)?;
            (Targets::Rig(t), pts)
        } else if d.mode == Mode::Edit {
            let (oi, mi) = crate::edit::active_mesh(d)?;
            let sel = crate::edit::selected_verts(d);
            if sel.is_empty() {
                return None;
            }
            let w = d.scene.world_matrix(oi);
            let orig = d.scene.meshes[mi].positions.clone();
            let pts: Vec<Vec3> = sel.iter().map(|&v| w.transform_point(orig[v as usize])).collect();
            let vo = VertOrig { mesh: mi, w, inv: w.inverse(), orig, sel: sel.clone(), moved: sel.iter().map(|&v| (v, 1.0)).collect() };
            (Targets::Verts(vo), pts)
        } else {
            let objs: Vec<ObjOrig> = d
                .scene
                .selected
                .iter()
                .filter(|&&i| i < d.scene.objects.len())
                .map(|&i| ObjOrig {
                    i,
                    t: d.scene.objects[i].transform,
                    world_loc: d.world_location(i),
                    parent_inv: d.scene.objects[i].parent.map(|p| d.scene.world_matrix(p).inverse()),
                })
                .collect();
            if objs.is_empty() {
                return None;
            }
            let pts = objs.iter().map(|o| o.world_loc).collect();
            (Targets::Objects(objs), pts)
        };
        let median = points.iter().fold(Vec3::ZERO, |a, &b| a + b) / points.len() as f32;
        let center = match pivot {
            Pivot::Cursor => d.cursor,
            Pivot::BoundingBoxCenter => {
                let mut b = sex_3d::Aabb::EMPTY;
                for &p in &points {
                    b.grow(p);
                }
                b.center()
            }
            Pivot::ActiveElement => active_point(d).unwrap_or(median),
            _ => median,
        };
        let mut m = Modal {
            mode,
            constraint: Constraint::None,
            space: Orientation::Global,
            basis: [Vec3::X, Vec3::Y, Vec3::Z],
            numeric: String::new(),
            snap_invert: false,
            start: mouse,
            mouse,
            center,
            angle_acc: 0.0,
            last_angle: 0.0,
            targets,
            pivot,
            release_confirms: false,
            undo_recorded: false,
            xf: Xf::Move(Vec3::ZERO),
            header: String::new(),
            roll: false,
        };
        m.last_angle = m.screen_angle(proj, mouse);
        m.update_weights(d);
        Some(m)
    }

    /// Constrain to `axis` (0..3) of `space`, e.g. a gizmo handle.
    pub fn with_axis(mut self, d: &Doc, proj: &Proj, axis: usize, plane: bool, space: Orientation) -> Modal {
        self.space = space;
        self.basis = orientation_basis(d, space, proj);
        self.constraint = if plane { Constraint::Plane(axis) } else { Constraint::Axis(axis) };
        self
    }

    fn screen_angle(&self, proj: &Proj, m: (f32, f32)) -> f32 {
        let c = proj.to_screen(self.center).unwrap_or((proj.cx, proj.cy));
        (-(m.1 - c.1)).atan2(m.0 - c.0)
    }

    /// Recompute proportional-editing weights (Edit Mode only).
    pub fn update_weights(&mut self, d: &Doc) {
        let Targets::Verts(v) = &mut self.targets else { return };
        v.moved = v.sel.iter().map(|&i| (i, 1.0)).collect();
        if !d.proportional {
            return;
        }
        let r = d.proportional_size.max(1e-4);
        let sel_w: Vec<Vec3> = v.sel.iter().map(|&i| v.w.transform_point(v.orig[i as usize])).collect();
        let selected: std::collections::HashSet<u32> = v.sel.iter().copied().collect();
        for (i, p) in v.orig.iter().enumerate() {
            if selected.contains(&(i as u32)) || d.edit_hidden.contains(&(i as u32)) {
                continue;
            }
            let pw = v.w.transform_point(*p);
            let dist = sel_w.iter().map(|s| s.distance(pw)).fold(f32::INFINITY, f32::min);
            if dist < r {
                let wt = d.falloff.weight(1.0 - dist / r);
                if wt > 0.0 {
                    v.moved.push((i as u32, wt));
                }
            }
        }
    }

    pub fn is_edit(&self) -> bool {
        matches!(self.targets, Targets::Verts(_))
    }

    /// A constraint key (X, Y, Z; `plane` with Shift). Pressing the same
    /// axis again switches to the local orientation, then frees it.
    pub fn constraint_key(&mut self, d: &Doc, proj: &Proj, axis: usize, plane: bool) {
        let same = match self.constraint {
            Constraint::Axis(a) => a == axis && !plane,
            Constraint::Plane(a) => a == axis && plane,
            _ => false,
        };
        let first = d.orientation;
        let second = if first == Orientation::Global { if d.mode == Mode::Edit { Orientation::Normal } else { Orientation::Local } } else { Orientation::Global };
        let space = if !same {
            first
        } else if self.space == first && first != second {
            second
        } else {
            self.constraint = Constraint::None;
            self.space = Orientation::Global;
            self.basis = [Vec3::X, Vec3::Y, Vec3::Z];
            return;
        };
        self.space = space;
        self.basis = orientation_basis(d, space, proj);
        self.constraint = if plane { Constraint::Plane(axis) } else { Constraint::Axis(axis) };
    }

    /// Feed a key. The caller applies the transform afterwards.
    pub fn key(&mut self, d: &mut Doc, proj: &Proj, key: Key, mods: Mods, text: Option<char>) -> KeyResult {
        let ch = match key {
            Key::Char(c) => Some(c),
            Key::Num(n) => char::from_digit(n as u32, 10),
            Key::NumDot => Some('.'),
            Key::NumMinus => Some('-'),
            _ => text,
        };
        match key {
            Key::Enter | Key::NumEnter | Key::Space => return KeyResult::Confirm,
            Key::Esc => return KeyResult::Cancel,
            Key::Backspace => {
                self.numeric.pop();
                return KeyResult::Continue;
            }
            Key::Tab if mods.shift => {
                self.snap_invert = !self.snap_invert;
                return KeyResult::Continue;
            }
            Key::PageUp | Key::NumPlus => {
                d.proportional_size *= 1.1;
                self.update_weights(d);
                return KeyResult::Continue;
            }
            Key::PageDown => {
                d.proportional_size /= 1.1;
                self.update_weights(d);
                return KeyResult::Continue;
            }
            _ => {}
        }
        let Some(c) = ch else { return KeyResult::Ignored };
        match c {
            '0'..='9' | '.' | '*' | '/' | '+' => self.numeric.push(c),
            '-' => {
                if let Some(rest) = self.numeric.strip_prefix('-') {
                    self.numeric = rest.to_string();
                } else {
                    self.numeric.insert(0, '-');
                }
            }
            'x' | 'y' | 'z' => {
                let axis = (c as u8 - b'x') as usize;
                self.constraint_key(d, proj, axis, mods.shift);
            }
            'g' | 'r' | 's' if !mods.ctrl => {
                let mode = match c {
                    'g' => TMode::Translate,
                    'r' => TMode::Rotate,
                    _ => TMode::Resize,
                };
                if mode != self.mode {
                    self.restore(d);
                    self.mode = mode;
                    self.numeric.clear();
                    self.constraint = Constraint::None;
                    self.space = Orientation::Global;
                    self.basis = [Vec3::X, Vec3::Y, Vec3::Z];
                    self.start = self.mouse;
                    self.angle_acc = 0.0;
                    self.last_angle = self.screen_angle(proj, self.mouse);
                }
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Continue
    }

    /// The pointer moved.
    pub fn mouse_to(&mut self, proj: &Proj, m: (f32, f32)) {
        self.mouse = m;
        let a = self.screen_angle(proj, m);
        let mut da = a - self.last_angle;
        while da > core::f32::consts::PI {
            da -= core::f32::consts::TAU;
        }
        while da < -core::f32::consts::PI {
            da += core::f32::consts::TAU;
        }
        self.angle_acc += da;
        self.last_angle = a;
    }

    fn axis_dir(&self) -> Option<Vec3> {
        match self.constraint {
            Constraint::Axis(a) | Constraint::Plane(a) => Some(self.basis[a]),
            Constraint::Dir(v) => Some(v),
            Constraint::None => None,
        }
    }

    /// Compute the transform from the pointer or the typed value.
    pub fn compute(&mut self, proj: &Proj, snap: bool) -> Xf {
        let snap = snap != self.snap_invert;
        let typed = parse_numeric(&self.numeric);
        let (dx, dy) = (self.mouse.0 - self.start.0, self.mouse.1 - self.start.1);
        let k = proj.world_per_pixel(self.center);
        let xf = match self.mode {
            TMode::Translate => {
                let delta = match (self.constraint, typed) {
                    (Constraint::None, Some(v)) => Vec3::X * v,
                    (Constraint::Plane(a), Some(v)) => self.basis[if a == 0 { 1 } else { 0 }] * v,
                    (_, Some(v)) => self.axis_dir().unwrap_or(Vec3::X) * v,
                    (Constraint::None, None) => {
                        let mut d = proj.right * (dx * k) - proj.up * (dy * k);
                        if snap {
                            d = crate::doc::round_grid(d);
                        }
                        d
                    }
                    (Constraint::Plane(a), None) => {
                        let n = self.basis[a];
                        let p0 = proj.on_plane(self.start.0, self.start.1, self.center, n);
                        let p1 = proj.on_plane(self.mouse.0, self.mouse.1, self.center, n);
                        let mut d = match (p0, p1) {
                            (Some(p0), Some(p1)) if proj.back.dot(n).abs() > 0.05 => p1 - p0,
                            _ => {
                                let v = proj.right * (dx * k) - proj.up * (dy * k);
                                v - n * v.dot(n)
                            }
                        };
                        if snap {
                            // Snap along the plane's two axes.
                            let mut out = Vec3::ZERO;
                            for (i, b) in self.basis.iter().enumerate() {
                                if i != a {
                                    out += *b * d.dot(*b).round();
                                }
                            }
                            d = out;
                        }
                        d
                    }
                    (_, None) => {
                        let dir = self.axis_dir().unwrap_or(Vec3::X).normalize();
                        let len = k * 50.0;
                        let s0 = proj.to_screen(self.center);
                        let s1 = proj.to_screen(self.center + dir * len);
                        let mut v = match (s0, s1) {
                            (Some(a), Some(b)) => {
                                let (sx, sy) = (b.0 - a.0, b.1 - a.1);
                                let l2 = sx * sx + sy * sy;
                                if l2 < 1e-2 {
                                    -dy * k
                                } else {
                                    (dx * sx + dy * sy) / l2 * len
                                }
                            }
                            _ => -dy * k,
                        };
                        if snap {
                            v = v.round();
                        }
                        dir * v
                    }
                };
                Xf::Move(delta)
            }
            TMode::Rotate => {
                let axis = self.axis_dir().unwrap_or(proj.back).normalize();
                let mut ang = match typed {
                    Some(v) => v.to_radians(),
                    None => {
                        let flip = if axis.dot(proj.back) < 0.0 { -1.0 } else { 1.0 };
                        self.angle_acc * flip
                    }
                };
                if snap && typed.is_none() {
                    let step = 5f32.to_radians();
                    ang = (ang / step).round() * step;
                }
                Xf::Rot(axis, ang)
            }
            TMode::Resize => {
                let f = match typed {
                    Some(v) => v,
                    None => {
                        let c = proj.to_screen(self.center).unwrap_or((proj.cx, proj.cy));
                        let d0 = ((self.start.0 - c.0).powi(2) + (self.start.1 - c.1).powi(2)).sqrt().max(1.0);
                        let d1 = ((self.mouse.0 - c.0).powi(2) + (self.mouse.1 - c.1).powi(2)).sqrt();
                        let mut f = d1 / d0;
                        if snap {
                            f = (f * 10.0).round() / 10.0;
                        }
                        f
                    }
                };
                // Scale factors along the global axes unless constrained.
                let fs = match self.constraint {
                    Constraint::Axis(a) => {
                        let mut v = [1.0; 3];
                        v[a] = f;
                        v
                    }
                    Constraint::Plane(a) => {
                        let mut v = [f; 3];
                        v[a] = 1.0;
                        v
                    }
                    _ => [f; 3],
                };
                Xf::Scale(fs)
            }
        };
        self.xf = xf;
        xf
    }

    fn basis_for_scale(&self) -> [Vec3; 3] {
        match self.constraint {
            Constraint::Axis(_) | Constraint::Plane(_) => self.basis,
            _ => [Vec3::X, Vec3::Y, Vec3::Z],
        }
    }

    /// Put everything back as it was.
    pub fn restore(&self, d: &mut Doc) {
        match &self.targets {
            Targets::Objects(objs) => {
                for o in objs {
                    if let Some(ob) = d.scene.objects.get_mut(o.i) {
                        ob.transform = o.t;
                    }
                }
            }
            Targets::Verts(v) => {
                if let Some(m) = d.scene.meshes.get_mut(v.mesh) {
                    if m.positions.len() == v.orig.len() {
                        m.positions.copy_from_slice(&v.orig);
                    }
                }
            }
            Targets::Rig(t) => t.restore(d),
        }
        d.touch();
    }

    /// Compute and write the transform into the document.
    pub fn apply(&mut self, d: &mut Doc, proj: &Proj) {
        let xf = self.compute(proj, d.snap);
        let center = self.center;
        let b = self.basis_for_scale();
        let scale_vec = |f: [f32; 3], v: Vec3, wt: f32| -> Vec3 {
            let mut out = Vec3::ZERO;
            for i in 0..3 {
                let fi = 1.0 + (f[i] - 1.0) * wt;
                out += b[i] * (v.dot(b[i]) * fi);
            }
            out
        };
        match &self.targets {
            Targets::Objects(objs) => {
                for o in objs {
                    let Some(ob) = d.scene.objects.get_mut(o.i) else { continue };
                    let to_local_vec = |v: Vec3| o.parent_inv.map_or(v, |m| m.transform_vector(v));
                    let to_local_pt = |p: Vec3| o.parent_inv.map_or(p, |m| m.transform_point(p));
                    let pivot = if self.pivot == Pivot::IndividualOrigins { o.world_loc } else { center };
                    let mut t = o.t;
                    match xf {
                        Xf::Move(dv) => t.location = o.t.location + to_local_vec(dv),
                        Xf::Rot(axis, ang) => {
                            let r = Quat::from_axis_angle(axis, ang);
                            t.location = to_local_pt(pivot + r.rotate(o.world_loc - pivot));
                            t.rotation = (r * o.t.rotation).normalize();
                        }
                        Xf::Scale(f) => {
                            t.location = to_local_pt(pivot + scale_vec(f, o.world_loc - pivot, 1.0));
                            let axes = [Vec3::X, Vec3::Y, Vec3::Z].map(|a| o.t.rotation.rotate(a));
                            let s = [o.t.scale.x, o.t.scale.y, o.t.scale.z];
                            let ns: Vec<f32> = (0..3)
                                .map(|j| {
                                    let sv = scale_vec(f, axes[j], 1.0);
                                    let sign = if sv.dot(axes[j]) < 0.0 { -1.0 } else { 1.0 };
                                    s[j] * sv.length() * sign
                                })
                                .collect();
                            t.scale = Vec3::new(ns[0], ns[1], ns[2]);
                        }
                    }
                    ob.transform = t;
                }
            }
            Targets::Rig(t) => {
                if self.roll {
                    if let Xf::Rot(_, ang) = xf {
                        t.roll(d, ang);
                    }
                } else {
                    let individual = self.pivot == Pivot::IndividualOrigins;
                    let affine = |pivot: Vec3| -> Mat4 {
                        let c = if individual { pivot } else { center };
                        match xf {
                            Xf::Move(dv) => Mat4::translation(dv),
                            Xf::Rot(axis, ang) => Mat4::translation(c) * Mat4::from_trs(Vec3::ZERO, Quat::from_axis_angle(axis, ang), Vec3::ONE) * Mat4::translation(-c),
                            Xf::Scale(f) => {
                                let cols = [Vec3::X, Vec3::Y, Vec3::Z].map(|a| scale_vec(f, a, 1.0));
                                Mat4::translation(c) * Mat4::from_mat3(sex_3d::math::Mat3::from_cols(cols[0], cols[1], cols[2]), Vec3::ZERO) * Mat4::translation(-c)
                            }
                        }
                    };
                    t.apply(d, &affine, individual, matches!(xf, Xf::Move(_)));
                }
            }
            Targets::Verts(v) => {
                let Some(m) = d.scene.meshes.get_mut(v.mesh) else { return };
                if m.positions.len() != v.orig.len() {
                    return;
                }
                for &(i, wt) in &v.moved {
                    let p = v.w.transform_point(v.orig[i as usize]);
                    let q = match xf {
                        Xf::Move(dv) => p + dv * wt,
                        Xf::Rot(axis, ang) => center + Quat::from_axis_angle(axis, ang * wt).rotate(p - center),
                        Xf::Scale(f) => center + scale_vec(f, p - center, wt),
                    };
                    m.positions[i as usize] = v.inv.transform_point(q);
                }
            }
        }
        self.header = self.header_text(d);
        d.touch();
    }

    /// Finish: record one undo step for the whole transform.
    pub fn confirm(&mut self, d: &mut Doc, proj: &Proj) {
        if !self.undo_recorded {
            self.restore(d);
            d.push_undo(if self.roll { "Roll" } else { self.mode.undo_label() });
        }
        self.apply(d, proj);
        if d.mode == Mode::Pose {
            crate::anim::autokey_bones(d);
        }
    }

    fn header_text(&self, d: &Doc) -> String {
        let typed = if self.numeric.is_empty() { None } else { Some(format!("[{}|]", self.numeric)) };
        let space = match self.space {
            Orientation::Global => "global",
            Orientation::Local => "local",
            Orientation::Normal => "normal",
            Orientation::Gimbal => "gimbal",
            Orientation::View => "view",
            Orientation::Cursor => "cursor",
            Orientation::Parent => "parent",
        };
        let axis_name = |a: usize| ["X", "Y", "Z"][a];
        let mut s = match self.xf {
            Xf::Move(dv) => match self.constraint {
                Constraint::Axis(a) => {
                    let len = dv.dot(self.basis[a]);
                    let v = typed.clone().unwrap_or_else(|| format!("{len:.4} m"));
                    format!("D: {v} ({}, {}, {}) along {space} {}", short(dv.x), short(dv.y), short(dv.z), axis_name(a))
                }
                Constraint::Dir(_) => {
                    let len = dv.length() * if self.axis_dir().is_some_and(|n| n.dot(dv) < 0.0) { -1.0 } else { 1.0 };
                    let v = typed.clone().unwrap_or_else(|| format!("{len:.4} m"));
                    format!("D: {v} ({}, {}, {}) along normal", short(dv.x), short(dv.y), short(dv.z))
                }
                Constraint::Plane(a) => {
                    format!("Dx: {:.4} m   Dy: {:.4} m   Dz: {:.4} m ({:.4} m) locking {space} {}", dv.x, dv.y, dv.z, dv.length(), axis_name(a))
                }
                Constraint::None => match &typed {
                    Some(t) => format!("Dx: {t}   Dy: 0 m   Dz: 0 m"),
                    None => format!("Dx: {:.4} m   Dy: {:.4} m   Dz: {:.4} m ({:.4} m)", dv.x, dv.y, dv.z, dv.length()),
                },
            },
            Xf::Rot(_, ang) if self.roll => {
                let v = typed.clone().unwrap_or_else(|| format!("{:.2}°", ang.to_degrees()));
                format!("Roll: {v}")
            }
            Xf::Rot(_, ang) => {
                let v = typed.clone().unwrap_or_else(|| format!("{:.2}°", ang.to_degrees()));
                match self.constraint {
                    Constraint::Axis(a) | Constraint::Plane(a) => format!("Rot: {v} along {space} {}", axis_name(a)),
                    _ => format!("Rot: {v}"),
                }
            }
            Xf::Scale(f) => {
                let body = match &typed {
                    Some(t) => format!("Scale: {t}"),
                    None => format!("Scale X: {:.4}  Y: {:.4}  Z: {:.4}", f[0], f[1], f[2]),
                };
                match self.constraint {
                    Constraint::Axis(a) => format!("{body} along {space} {}", axis_name(a)),
                    Constraint::Plane(a) => format!("{body} locking {space} {}", axis_name(a)),
                    _ => body,
                }
            }
        };
        if d.snap != self.snap_invert {
            s.push_str("   (Snap)");
        }
        if d.proportional && self.is_edit() {
            s.push_str(&format!("   Proportional size: {:.2}", d.proportional_size));
        }
        s
    }
}

/// World position of the active element (object origin or edit element).
fn active_point(d: &Doc) -> Option<Vec3> {
    if d.mode == Mode::Edit {
        let (oi, mi) = crate::edit::active_mesh(d)?;
        let m = &d.scene.meshes[mi];
        let w = d.scene.world_matrix(oi);
        let p = match d.edit.active? {
            Elem::Vert(v) => *m.positions.get(v as usize)?,
            Elem::Edge(a, b) => (*m.positions.get(a as usize)? + *m.positions.get(b as usize)?) * 0.5,
            Elem::Face(f) => {
                if f as usize >= m.faces.len() {
                    return None;
                }
                m.face_center(f as usize)
            }
        };
        Some(w.transform_point(p))
    } else {
        d.active().map(|a| d.world_location(a))
    }
}
