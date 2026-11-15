// SPDX-License-Identifier: GPL-3.0-only
//! The 3D Viewport, laid out and keyed like Blender 4.x's.
//!
//! Header: mode, the View/Select/Add/Object menus (Edit Mode: select modes
//! and View/Select/Add/Mesh/Vertex/Edge/Face/UV), the transform orientation,
//! pivot, snapping and proportional editing in the middle, and the gizmo,
//! overlay, X-ray and the four shading buttons at the right.
//!
//! Body: Solid shading from sex-3d's rasterizer, Wireframe, and Material
//! Preview / Rendered from its progressive path tracer; overlays (the 3D
//! cursor, object origins, Edit Mode's vertices/edges/faces, the camera frame
//! in camera view); the toolbar (T) with the active tool's gizmo; the sidebar
//! (N); the navigation gizmo with its zoom/pan/camera/ortho buttons.
//!
//! Interaction: middle-drag orbit (Shift pan, Ctrl zoom), wheel zoom, the
//! numpad views, click and box select, Shift+right-click for the 3D cursor,
//! right-click context menus, the modal G/R/S transforms and Edit Mode's
//! modal tools (extrude, inset, bevel, loop cut, knife). Alt is not delivered
//! by SexOS's Shell yet: every Alt shortcut also has a menu entry.
pub mod icons;
pub mod menus;
mod overlay;
pub mod proj;
pub mod raster;
pub mod render;
pub mod sidebar;
#[cfg(test)]
mod tests;
mod widgets;

use crate::doc::{is_view_op, split_view_tag, BoxMode, Doc, Mode, Tool};
use crate::draw::Rect;
use crate::edit;
use crate::input::{Button, Input, Key, Mods};
use crate::transform::{self, Constraint, KeyResult, TMode};
use crate::ui::{Id, MenuItem, Op, Popup, Ui};
use proj::Proj;
use sex_3d::render::viewport::{ViewCamera, ViewportScene};
use sex_3d::scene::{ObjectData, Scene};
use sex_3d::{Aabb, Mesh, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shading {
    Wireframe,
    Solid,
    Material,
    Rendered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideTab {
    Item,
    Tool,
    View,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshToolKind {
    Inset,
    Bevel,
}

/// A modal Edit Mode tool that rebuilds the mesh from its original on every
/// pointer move (Inset, Bevel).
pub struct MeshTool {
    pub kind: MeshToolKind,
    mi: usize,
    orig: Mesh,
    faces: Vec<u32>,
    edges: Vec<[u32; 2]>,
    center: Vec3,
    start: (f32, f32),
    pub numeric: String,
    pub segments: u32,
    pub value: f32,
}

/// What the viewport is in the middle of.
pub enum Modal {
    None,
    Orbit,
    Pan,
    Zoom,
    /// Left button down on the view: a click or the start of a box.
    Press { x: i32, y: i32, shift: bool, ctrl: bool },
    Box { x0: i32, y0: i32, x1: i32, y1: i32, mode: BoxMode },
    /// B was pressed: the next drag draws a box.
    BoxWait,
    Transform(Box<transform::Modal>),
    MeshTool(Box<MeshTool>),
    LoopCut { edge: Option<[u32; 2]>, cuts: u32, preview: Vec<(Vec3, Vec3)> },
    Knife { pts: Vec<(f32, f32)> },
    Annotate,
    Measure,
}

pub struct State {
    pub cam: ViewCamera,
    /// Viewport focal length (Blender's 50 mm on a 72 mm sensor).
    pub lens: f32,
    pub clip_start: f32,
    pub clip_end: f32,
    pub shading: Shading,
    pub xray: bool,
    pub show_gizmo: bool,
    pub show_nav_gizmo: bool,
    pub show_tool_gizmo: bool,
    pub show_overlays: bool,
    pub show_floor: bool,
    pub show_text: bool,
    pub show_cursor: bool,
    pub show_stats: bool,
    pub show_wire: bool,
    pub backface_culling: bool,
    pub show_toolbar: bool,
    pub show_sidebar: bool,
    pub side_tab: SideTab,
    pub sidebar_w: i32,
    /// Name of the axis view ("Top", "Front"...), if the view is one.
    pub axis_view: Option<&'static str>,
    /// Perspective to return to when orbiting out of an axis view (Blender's
    /// Auto Perspective).
    persp_before_axis: Option<bool>,
    /// Looking through the scene camera (Numpad 0).
    pub camera_view: bool,
    pub modal: Modal,
    last: (i32, i32),
    /// Viewport geometry: (generation, everything, everything but meshes).
    cache: Option<(u64, ViewportScene, ViewportScene)>,
    /// World-space points and edges of every visible mesh object.
    world: Option<(u64, Vec<WorldObj>)>,
    buf: Vec<u8>,
    buf2: Vec<u8>,
    edit_view: Option<(u64, [u32; 8], Rect, raster::EditView)>,
    render: Option<render::RenderView>,
    /// Material Preview / Rendered still adding samples.
    pub refining: bool,
    pub annotations: Vec<Vec<Vec3>>,
    pub measure: Option<(Vec3, Vec3)>,
    pub rect: Rect,
    pub(crate) undo_widget: Option<Id>,
    pub panels: [bool; 6],
    pub median_global: bool,
    dims: Option<(u64, usize, Option<Vec3>)>,
}

pub struct WorldObj {
    pub index: usize,
    pub points: Vec<Vec3>,
    pub edges: Vec<[u32; 2]>,
    pub tris: Vec<[u32; 3]>,
}

impl Default for State {
    fn default() -> Self {
        State {
            cam: ViewCamera::default(),
            lens: 50.0,
            clip_start: 0.01,
            clip_end: 1000.0,
            shading: Shading::Solid,
            xray: false,
            show_gizmo: true,
            show_nav_gizmo: true,
            show_tool_gizmo: true,
            show_overlays: true,
            show_floor: true,
            show_text: true,
            show_cursor: true,
            show_stats: false,
            show_wire: false,
            backface_culling: false,
            show_toolbar: true,
            show_sidebar: false,
            side_tab: SideTab::Item,
            sidebar_w: 250,
            axis_view: None,
            persp_before_axis: None,
            camera_view: false,
            modal: Modal::None,
            last: (0, 0),
            cache: None,
            world: None,
            buf: Vec::new(),
            buf2: Vec::new(),
            edit_view: None,
            render: None,
            refining: false,
            annotations: Vec::new(),
            measure: None,
            rect: Rect::default(),
            undo_widget: None,
            panels: [true; 6],
            median_global: false,
            dims: None,
        }
    }
}

impl State {
    /// Size of the active object's evaluated mesh (object space), cached.
    pub fn dims_cache(&mut self, d: &Doc, a: usize) -> Option<Vec3> {
        if let Some((g, i, v)) = self.dims {
            if g == d.generation && i == a {
                return v;
            }
        }
        let v = d.scene.evaluated_mesh(a).map(|m| m.bounds()).filter(|b| !b.is_empty()).map(|b| b.size());
        self.dims = Some((d.generation, a, v));
        v
    }
    fn xray_on(&self) -> bool {
        self.xray || self.shading == Shading::Wireframe
    }
}

pub fn is_modal(s: &State) -> bool {
    !matches!(s.modal, Modal::None)
}

/// Whether the viewport wants more frames without input (progressive render).
pub fn wants_tick(s: &State) -> bool {
    s.refining
}

fn cam_key(c: &ViewCamera) -> [u32; 8] {
    [c.target.x, c.target.y, c.target.z, c.distance, c.yaw, c.pitch, c.fov_y, if c.ortho { 1.0 } else { 0.0 }].map(f32::to_bits)
}

// ---------------------------------------------------------------- header

/// Tag the view operators of a menu with a point in this area, so the
/// viewport that opened it runs them.
pub fn tag(items: &mut [MenuItem], at: (i32, i32)) {
    for it in items {
        match it {
            MenuItem::Op(_, _, op) | MenuItem::Check(_, _, op) if is_view_op(&op.id) => {
                op.arg = format!("@{},{};{}", at.0, at.1, op.arg);
            }
            MenuItem::Sub(_, v) => tag(v, at),
            _ => {}
        }
    }
}

fn tag_open(ui: &mut Ui, at: (i32, i32)) {
    if let Some(p) = &mut ui.st.open_popup {
        tag(&mut p.items, at);
    }
}

/// Header text shown in place of the header while a modal tool runs.
fn modal_header(s: &State) -> Option<String> {
    match &s.modal {
        Modal::Transform(m) => Some(m.header.clone()),
        Modal::MeshTool(m) => Some(match m.kind {
            MeshToolKind::Inset => format!("Thickness: {}", if m.numeric.is_empty() { format!("{:.4} m", m.value) } else { format!("[{}|]", m.numeric) }),
            MeshToolKind::Bevel => format!(
                "Width: {}   Segments: {}   (wheel or Page Up/Down for segments)",
                if m.numeric.is_empty() { format!("{:.4} m", m.value) } else { format!("[{}|]", m.numeric) },
                m.segments
            ),
        }),
        Modal::LoopCut { cuts, .. } => Some(format!("Number of Cuts: {cuts}   (wheel or Page Up/Down; click to cut, Esc to cancel)")),
        Modal::Knife { pts } => Some(format!("Knife: click to add cut points ({}), Enter to cut, Esc to cancel", pts.len())),
        Modal::BoxWait => Some("Box Select: drag to select".into()),
        _ => None,
    }
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    widgets::header(s, d, ui, hr);
}

// ---------------------------------------------------------------- camera

/// Field of view from the lens (Blender's viewport uses a 72 mm sensor
/// across the longer side).
fn fov_for(lens: f32, r: Rect) -> f32 {
    let aspect = r.w.max(1) as f32 / r.h.max(1) as f32;
    let t = 36.0 / lens.max(1.0);
    let tv = if aspect >= 1.0 { t / aspect } else { t };
    2.0 * tv.atan()
}

/// In camera view: the camera frame's half size in pixels.
pub fn camera_frame(s: &State, d: &Doc, r: Rect) -> Option<(f32, f32)> {
    let ci = d.scene.active_camera.filter(|&c| c < d.scene.objects.len())?;
    let cam = match &d.scene.objects[ci].data {
        ObjectData::Camera(c) => *c,
        _ => return None,
    };
    let tv = (cam.fov_y * 0.5).tan();
    let view_t = (s.cam.fov_y * 0.5).tan();
    let hh = tv / view_t * r.h as f32 * 0.5;
    Some((hh * 16.0 / 9.0, hh))
}

/// Keep the view camera in step: the lens, and the scene camera in camera view.
fn prepare_camera(s: &mut State, d: &Doc, r: Rect) {
    if s.camera_view {
        let Some(ci) = d.scene.active_camera.filter(|&c| c < d.scene.objects.len()) else {
            s.camera_view = false;
            return prepare_camera(s, d, r);
        };
        let ObjectData::Camera(cam) = &d.scene.objects[ci].data else {
            s.camera_view = false;
            return prepare_camera(s, d, r);
        };
        let m = d.scene.world_matrix(ci);
        let eye = m.transform_point(Vec3::ZERO);
        let back = m.transform_vector(Vec3::Z).normalize();
        s.cam.pitch = back.z.clamp(-1.0, 1.0).asin();
        s.cam.yaw = back.y.atan2(back.x);
        s.cam.ortho = false;
        if s.cam.distance <= 0.0 {
            s.cam.distance = 10.0;
        }
        s.cam.target = eye - back * s.cam.distance;
        // Fit the 16:9 frame inside the view with a margin.
        let tv = (cam.fov_y * 0.5).tan();
        let aspect = r.w.max(1) as f32 / r.h.max(1) as f32;
        let need = tv.max(tv * 16.0 / 9.0 / aspect) / 0.8;
        s.cam.fov_y = 2.0 * need.atan();
    } else {
        s.cam.fov_y = fov_for(s.lens, r);
    }
}

fn leave_special_views(s: &mut State) {
    s.camera_view = false;
    if s.axis_view.take().is_some() {
        if let Some(p) = s.persp_before_axis.take() {
            s.cam.ortho = p;
        }
    }
}

pub fn set_axis_view(s: &mut State, name: &str) {
    use core::f32::consts::{FRAC_PI_2, PI};
    let (yaw, pitch, label): (f32, f32, &'static str) = match name {
        "FRONT" => (-FRAC_PI_2, 0.0, "Front"),
        "BACK" => (FRAC_PI_2, 0.0, "Back"),
        "RIGHT" => (0.0, 0.0, "Right"),
        "LEFT" => (PI, 0.0, "Left"),
        "TOP" => (-FRAC_PI_2, FRAC_PI_2, "Top"),
        "BOTTOM" => (-FRAC_PI_2, -FRAC_PI_2, "Bottom"),
        _ => return,
    };
    s.camera_view = false;
    if s.persp_before_axis.is_none() {
        s.persp_before_axis = Some(s.cam.ortho);
    }
    s.cam.yaw = yaw;
    s.cam.pitch = pitch;
    s.cam.ortho = true;
    s.axis_view = Some(label);
}

fn opposite_axis(name: &str) -> &'static str {
    match name {
        "Front" => "BACK",
        "Back" => "FRONT",
        "Right" => "LEFT",
        "Left" => "RIGHT",
        "Top" => "BOTTOM",
        _ => "TOP",
    }
}

fn orbit_by(s: &mut State, dyaw: f32, dpitch: f32) {
    leave_special_views(s);
    s.cam.yaw += dyaw;
    s.cam.pitch = (s.cam.pitch + dpitch).clamp(-1.5707, 1.5707);
}

fn pan_by(s: &mut State, dx: f32, dy: f32, r: Rect) {
    s.camera_view = false;
    let p = Proj::new(&s.cam, r);
    let k = p.world_per_pixel(s.cam.target);
    s.cam.target = s.cam.target - s.cam.right() * (dx * k) + s.cam.up() * (dy * k);
}

fn zoom_by(s: &mut State, f: f32) {
    s.camera_view = false;
    s.cam.distance = (s.cam.distance * f).clamp(1e-3, 1e5);
}

/// Bounds of what is selected (objects, or Edit Mode's vertices).
fn selected_bounds(d: &Doc) -> Aabb {
    let mut b = Aabb::EMPTY;
    if d.mode == Mode::Edit {
        if let Some((oi, mi)) = edit::active_mesh(d) {
            let w = d.scene.world_matrix(oi);
            for v in edit::selected_verts(d) {
                b.grow(w.transform_point(d.scene.meshes[mi].positions[v as usize]));
            }
        }
        return b;
    }
    for &i in &d.scene.selected {
        b = b.union(object_bounds(&d.scene, i));
    }
    b
}

fn object_bounds(s: &Scene, i: usize) -> Aabb {
    if i >= s.objects.len() {
        return Aabb::EMPTY;
    }
    if let Some((m, _)) = s.world_mesh(i) {
        if !m.bounds().is_empty() {
            return m.bounds();
        }
    }
    let o = s.world_matrix(i).transform_point(Vec3::ZERO);
    Aabb { min: o - Vec3::splat(1.0), max: o + Vec3::splat(1.0) }
}

fn frame_bounds(s: &mut State, b: Aabb) {
    if b.is_empty() {
        return;
    }
    s.camera_view = false;
    // A single point still gets a sensible distance.
    let b = if b.size().length() < 1e-4 { Aabb { min: b.min - Vec3::splat(0.5), max: b.max + Vec3::splat(0.5) } } else { b };
    s.cam.frame(b);
}

fn frame_all(s: &mut State, d: &Doc) {
    let mut b = Aabb::EMPTY;
    for i in 0..d.scene.objects.len() {
        if d.scene.is_visible(i) {
            b = b.union(object_bounds(&d.scene, i));
        }
    }
    frame_bounds(s, b);
}

/// Put the active camera where the view is (Ctrl+Alt+Numpad 0).
fn camera_to_view(s: &State, d: &mut Doc) {
    let Some(ci) = d.scene.active_camera.filter(|&c| c < d.scene.objects.len()) else {
        d.report = "No active camera".into();
        return;
    };
    d.push_undo("Align Camera to View");
    let (r, u, b) = (s.cam.right(), s.cam.up(), s.cam.back());
    let e = s.cam.eye();
    let m = sex_3d::Mat4::from_cols_array([r.x, r.y, r.z, 0.0, u.x, u.y, u.z, 0.0, b.x, b.y, b.z, 0.0, e.x, e.y, e.z, 1.0]);
    let local = match d.scene.objects[ci].parent {
        Some(p) => d.scene.world_matrix(p).inverse() * m,
        None => m,
    };
    d.scene.objects[ci].transform = sex_3d::Transform::from_matrix(&local);
    d.touch();
}

// ---------------------------------------------------------------- caches

fn ensure_cache(s: &mut State, d: &mut Doc) {
    if s.cache.as_ref().is_some_and(|c| c.0 == d.generation) {
        return;
    }
    // In Edit Mode the edited object shows its edit overlay, not an outline.
    let saved = d.scene.selected.clone();
    if d.mode == Mode::Edit {
        if let Some(a) = d.active() {
            d.scene.selected.retain(|&i| i != a);
        }
    }
    let full = ViewportScene::new(&d.scene);
    let shell = Scene {
        name: String::new(),
        objects: d
            .scene
            .objects
            .iter()
            .map(|o| {
                let mut o = o.clone();
                if matches!(o.data, ObjectData::Mesh(_)) {
                    o.visible = false;
                    o.modifiers.clear();
                }
                o
            })
            .collect(),
        collections: d.scene.collections.clone(),
        meshes: Vec::new(),
        mesh_names: Vec::new(),
        materials: Vec::new(),
        images: Vec::new(),
        world: d.scene.world.clone(),
        active_camera: d.scene.active_camera,
        active_object: d.scene.active_object,
        selected: d.scene.selected.clone(),
        frame_start: d.scene.frame_start,
        frame_end: d.scene.frame_end,
        fps: d.scene.fps,
        frame_current: d.scene.frame_current,
        node_groups: Vec::new(),
    };
    let empty = ViewportScene::new(&shell);
    d.scene.selected = saved;
    s.cache = Some((d.generation, full, empty));
}

fn ensure_world(s: &mut State, d: &Doc) {
    if s.world.as_ref().is_some_and(|w| w.0 == d.generation) {
        return;
    }
    let mut out = Vec::new();
    for i in 0..d.scene.objects.len() {
        if !d.scene.is_visible(i) {
            continue;
        }
        if let Some((m, _)) = d.scene.world_mesh(i) {
            let tris = m.triangulate().into_iter().map(|t| t.verts).collect();
            out.push(WorldObj { index: i, edges: m.edges(), points: m.positions, tris });
        }
    }
    s.world = Some((d.generation, out));
}

fn edit_view<'a>(s: &'a mut State, d: &Doc, proj: &Proj) -> Option<&'a raster::EditView> {
    if d.mode != Mode::Edit {
        return None;
    }
    let key = cam_key(&s.cam);
    let fresh = s.edit_view.as_ref().is_some_and(|e| e.0 == d.generation && e.1 == key && e.2 == proj.rect);
    if !fresh {
        let ev = raster::EditView::build(d, proj)?;
        s.edit_view = Some((d.generation, key, proj.rect, ev));
    }
    s.edit_view.as_ref().map(|e| &e.3)
}

/// The nearest surface point under (x, y), or the point at the 3D cursor's
/// depth (where Blender puts the cursor when it misses everything).
fn surface_point(s: &mut State, d: &Doc, proj: &Proj, x: f32, y: f32) -> Vec3 {
    ensure_world(s, d);
    let (o, dir) = proj.ray(x, y);
    let mut best = f32::INFINITY;
    if let Some((_, objs)) = &s.world {
        for w in objs {
            for t in &w.tris {
                let [a, b, c] = t.map(|i| w.points[i as usize]);
                if let Some(tt) = ray_tri(o, dir, a, b, c) {
                    best = best.min(tt);
                }
            }
        }
    }
    if best.is_finite() {
        o + dir * best
    } else {
        proj.at_depth(x, y, d.cursor)
    }
}

fn ray_tri(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = o - a;
    let u = tv.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = tv.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-6).then_some(t)
}

// ---------------------------------------------------------------- view ops

fn consume_view_ops(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if d.view_ops.is_empty() {
        return;
    }
    let ops = core::mem::take(&mut d.view_ops);
    let mut keep = Vec::new();
    for op in ops {
        let (pt, arg) = split_view_tag(&op.arg);
        if pt.is_none_or(|(x, y)| r.contains(x, y)) {
            let arg = arg.to_string();
            run_view_op(s, d, ui, &op.id, &arg, r);
        } else {
            keep.push(op);
        }
    }
    d.view_ops.extend(keep);
}

fn mouse_in(ui: &Ui, r: Rect) -> (f32, f32) {
    let (x, y) = ui.mouse();
    if r.contains(x, y) {
        (x as f32, y as f32)
    } else {
        let (cx, cy) = r.center();
        (cx as f32 + 80.0, cy as f32)
    }
}

fn run_view_op(s: &mut State, d: &mut Doc, ui: &mut Ui, id: &str, arg: &str, r: Rect) {
    let m = mouse_in(ui, r);
    match id {
        "uv.project_from_view" => {
            let p = Proj::new(&s.cam, r);
            let f = move |w: Vec3| p.to_screen(w);
            crate::uvedit::project_from_view(d, Some((&f, (r.x as f32, r.y as f32, r.w as f32, r.h as f32))), arg.contains("BOUNDS"));
        }
        "view3d.view_all" => frame_all(s, d),
        "view3d.view_selected" => frame_bounds(s, selected_bounds(d)),
        "view3d.view_axis" => set_axis_view(s, arg),
        "view3d.view_camera" => {
            if d.scene.active_camera.is_some() {
                s.camera_view = !s.camera_view;
                if s.camera_view {
                    s.axis_view = None;
                }
            } else {
                d.report = "No active camera".into();
            }
        }
        "view3d.view_persportho" => {
            s.cam.ortho = !s.cam.ortho;
            s.persp_before_axis = None;
        }
        "view3d.view_orbit" => {
            let st = 15f32.to_radians();
            match arg {
                "LEFT" => orbit_by(s, st, 0.0),
                "RIGHT" => orbit_by(s, -st, 0.0),
                "UP" => orbit_by(s, 0.0, st),
                "DOWN" => orbit_by(s, 0.0, -st),
                _ => {
                    let keep = s.axis_view;
                    s.cam.yaw += core::f32::consts::PI;
                    if let Some(a) = keep {
                        set_axis_view(s, opposite_axis(a));
                    }
                }
            }
        }
        "view3d.view_pan" => match arg {
            "LEFT" => pan_by(s, -40.0, 0.0, r),
            "RIGHT" => pan_by(s, 40.0, 0.0, r),
            "UP" => pan_by(s, 0.0, -40.0, r),
            _ => pan_by(s, 0.0, 40.0, r),
        },
        "view3d.zoom" => zoom_by(s, if arg == "OUT" { 1.0 / 0.85 } else { 0.85 }),
        "view3d.toggle_xray" => s.xray = !s.xray,
        "view3d.toolbar" => s.show_toolbar = !s.show_toolbar,
        "view3d.sidebar" => s.show_sidebar = !s.show_sidebar,
        "view3d.camera_to_view" => camera_to_view(s, d),
        "view3d.object_as_camera" => {
            if let Some(a) = d.active().filter(|&a| matches!(d.scene.objects[a].data, ObjectData::Camera(_))) {
                d.scene.active_camera = Some(a);
                s.camera_view = true;
            } else {
                d.report = "Active object is not a camera".into();
            }
        }
        "view3d.view_center_cursor" => {
            s.cam.target = d.cursor;
            frame_all(s, d);
            s.cam.target = d.cursor;
        }
        "view3d.select_box" => s.modal = Modal::BoxWait,
        "view3d.shading" => {
            s.shading = match arg {
                "WIREFRAME" => Shading::Wireframe,
                "MATERIAL" => Shading::Material,
                "RENDERED" => Shading::Rendered,
                _ => Shading::Solid,
            }
        }
        "view3d.overlay" => {
            let f = match arg {
                "floor" => &mut s.show_floor,
                "text" => &mut s.show_text,
                "cursor" => &mut s.show_cursor,
                "stats" => &mut s.show_stats,
                "wire" => &mut s.show_wire,
                "gizmo_nav" => &mut s.show_nav_gizmo,
                "gizmo_tool" => &mut s.show_tool_gizmo,
                "backface" => &mut s.backface_culling,
                _ => &mut s.show_overlays,
            };
            *f = !*f;
        }
        "transform.translate" => start_transform(s, d, TMode::Translate, m, r),
        "transform.rotate" => start_transform(s, d, TMode::Rotate, m, r),
        "transform.resize" => start_transform(s, d, TMode::Resize, m, r),
        "object.duplicate_move" | "object.duplicate_move_linked" => {
            d.run(&Op::new(if id == "object.duplicate_move" { "object.duplicate" } else { "object.duplicate_linked" }));
            start_transform(s, d, TMode::Translate, m, r);
            mark_undo_recorded(s);
        }
        "mesh.duplicate_move" => {
            if edit::duplicate(d) {
                start_transform(s, d, TMode::Translate, m, r);
                mark_undo_recorded(s);
            }
        }
        "mesh.extrude_region_move" => extrude_move(s, d, m, r),
        "mesh.extrude_faces_indiv_move" => {
            if edit::extrude_individual(d) {
                start_transform(s, d, TMode::Translate, m, r);
                mark_undo_recorded(s);
            }
        }
        "mesh.extrude_region_shrink_fatten" => extrude_move(s, d, m, r),
        "mesh.extrude_manifold" => {
            if d.mode == Mode::Edit {
                if let Some(n) = crate::edit_ops::extrude_manifold(d) {
                    move_after_op(s, d, m, r, Some(n));
                }
            }
        }
        "mesh.rip_move" | "mesh.rip_move_fill" | "mesh.rip_edge_move" => {
            if d.mode == Mode::Edit {
                let ok = if id == "mesh.rip_edge_move" { crate::edit_ops::rip_edge(d) } else { crate::edit_ops::rip(d, id == "mesh.rip_move_fill") };
                if ok {
                    move_after_op(s, d, m, r, None);
                }
            }
        }
        "mesh.inset" => start_mesh_tool(s, d, MeshToolKind::Inset, m, r),
        "mesh.bevel" => start_mesh_tool(s, d, MeshToolKind::Bevel, m, r),
        "mesh.loopcut_slide" => {
            if d.mode == Mode::Edit {
                s.modal = Modal::LoopCut { edge: None, cuts: 1, preview: Vec::new() };
                loopcut_hover(s, d, m, r);
            }
        }
        "mesh.knife_tool" => {
            if d.mode == Mode::Edit {
                s.modal = Modal::Knife { pts: Vec::new() };
            }
        }
        "object.move_to_collection_menu" => {
            let mut p = Popup::new("Move to Collection", menus::collection_items(d), m.0 as i32 - 20, m.1 as i32 - 10);
            p.owner = Rect::default();
            d.menu = Some(p);
        }
        _ => d.report = format!("{id} is not available yet"),
    }
}

fn start_transform(s: &mut State, d: &mut Doc, mode: TMode, m: (f32, f32), r: Rect) {
    if d.mode != Mode::Object && d.mode != Mode::Edit {
        return;
    }
    let p = Proj::new(&s.cam, r);
    match transform::Modal::start(d, mode, &p, m) {
        Some(mut t) => {
            t.apply(d, &p);
            s.modal = Modal::Transform(Box::new(t));
        }
        None => d.report = "Nothing selected".into(),
    }
}

/// The move just started follows an operator that recorded its own undo step.
fn mark_undo_recorded(s: &mut State) {
    if let Modal::Transform(t) = &mut s.modal {
        t.undo_recorded = true;
    }
}

fn extrude_move(s: &mut State, d: &mut Doc, m: (f32, f32), r: Rect) {
    if d.mode != Mode::Edit {
        return;
    }
    let Some(dir) = edit::extrude(d) else { return };
    let p = Proj::new(&s.cam, r);
    if let Some(mut t) = transform::Modal::start(d, TMode::Translate, &p, m) {
        if let Some(n) = dir {
            t.constraint = Constraint::Dir(n);
        }
        t.undo_recorded = true;
        t.apply(d, &p);
        s.modal = Modal::Transform(Box::new(t));
    }
}

/// Start a move of what an edit operator just made (it recorded its own
/// undo step), along `dir` when given.
fn move_after_op(s: &mut State, d: &mut Doc, m: (f32, f32), r: Rect, dir: Option<Vec3>) {
    let p = Proj::new(&s.cam, r);
    if let Some(mut t) = transform::Modal::start(d, TMode::Translate, &p, m) {
        if let Some(n) = dir {
            t.constraint = Constraint::Dir(n);
        }
        t.undo_recorded = true;
        t.apply(d, &p);
        s.modal = Modal::Transform(Box::new(t));
    }
}

fn start_mesh_tool(s: &mut State, d: &mut Doc, kind: MeshToolKind, m: (f32, f32), r: Rect) {
    if d.mode != Mode::Edit {
        return;
    }
    let Some((_, mi)) = edit::active_mesh(d) else { return };
    let faces = edit::selected_faces(d);
    let edges = edit::selected_edges(d);
    if kind == MeshToolKind::Inset && faces.is_empty() {
        d.report = "Select faces to inset".into();
        return;
    }
    if kind == MeshToolKind::Bevel && edges.is_empty() {
        d.report = "Select edges to bevel".into();
        return;
    }
    let Some(center) = d.selection_center() else { return };
    let _ = r;
    s.modal = Modal::MeshTool(Box::new(MeshTool { kind, mi, orig: d.scene.meshes[mi].clone(), faces, edges, center, start: m, numeric: String::new(), segments: 1, value: 0.0 }));
}

// ---------------------------------------------------------------- body

/// Rectangles of the regions drawn over the view.
pub struct Layout {
    pub toolbar: Rect,
    pub sidebar: Rect,
    pub nav: Rect,
}

fn layout(s: &State, d: &Doc, r: Rect) -> Layout {
    let toolbar = if s.show_toolbar { widgets::toolbar_rect(d, r) } else { Rect::default() };
    let sidebar = if s.show_sidebar { Rect::new(r.right() - s.sidebar_w, r.y, s.sidebar_w, r.h) } else { Rect::default() };
    let right = if s.show_sidebar { sidebar.x } else { r.right() };
    let nav = if s.show_gizmo && s.show_nav_gizmo { Rect::new(right - 110, r.y + 8, 104, 230) } else { Rect::default() };
    Layout { toolbar, sidebar, nav }
}

impl Layout {
    fn over(&self, x: i32, y: i32) -> bool {
        self.toolbar.contains(x, y) || self.sidebar.contains(x, y) || widgets::nav_hit_area(self.nav, x, y)
    }
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if r.is_empty() {
        return;
    }
    s.rect = r;
    prepare_camera(s, d, r);
    consume_view_ops(s, d, ui, r);
    prepare_camera(s, d, r);
    if d.mode != Mode::Edit && matches!(s.modal, Modal::MeshTool(_) | Modal::LoopCut { .. } | Modal::Knife { .. }) {
        s.modal = Modal::None;
    }
    let lay = layout(s, d, r);
    if let Some(inp) = ui.input {
        let busy = ui.st.active.is_some() || ui.st.editing.is_some();
        let header_before = modal_header(s);
        if is_modal(s) {
            handle_modal(s, d, ui, inp, r);
        } else if !busy && !inp.pos().is_some_and(|(x, y)| lay.over(x, y)) {
            handle_event(s, d, ui, inp, r);
        }
        // The header was drawn before this event: have the window draw again.
        if modal_header(s) != header_before {
            d.queue.push(Op::new("wm.redraw"));
        }
    }
    prepare_camera(s, d, r);
    // Rendered shading adds one sample per frame.
    s.refining = false;
    if matches!(s.shading, Shading::Material | Shading::Rendered) {
        s.refining = render::RenderView::refresh(&mut s.render, &d.scene, d.generation, &s.cam, r.w, r.h, s.shading == Shading::Material);
    } else {
        s.render = None;
    }
    let sig = signature(s, d);
    let lay = layout(s, d, r);
    draw_all(s, d, ui, r, &lay);
    prepare_camera(s, d, r);
    if signature(s, d) != sig {
        // A widget changed the scene or the view: draw again so the frame is current.
        let keep = ui.input.take();
        let lay = layout(s, d, r);
        draw_all(s, d, ui, r, &lay);
        ui.input = keep;
    }
}

fn signature(s: &State, d: &Doc) -> (u64, [u32; 8], u32, [u32; 3], u8) {
    let flags = [s.show_toolbar, s.show_sidebar, s.xray, s.camera_view, s.show_overlays, s.show_gizmo, s.show_floor, s.show_wire]
        .iter()
        .enumerate()
        .fold(0u32, |a, (i, &b)| a | (b as u32) << i)
        | (s.shading as u32) << 10
        | (d.tool as u32) << 14
        | (s.side_tab as u32) << 20;
    (d.generation, cam_key(&s.cam), flags, d.cursor.to_array().map(f32::to_bits), d.mode as u8)
}

fn draw_all(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect, lay: &Layout) {
    let proj = Proj::new(&s.cam, r);
    overlay::scene_image(s, d, ui, r);
    if s.camera_view {
        overlay::camera_frame(s, d, ui, r);
    }
    if s.show_overlays {
        if d.mode == Mode::Edit {
            overlay::edit_overlay(s, d, ui, &proj);
        } else {
            overlay::origins(d, ui, &proj);
        }
        if s.show_cursor {
            overlay::cursor(ui, &proj, d.cursor);
        }
        overlay::annotations(s, ui, &proj);
    }
    overlay::modal_overlay(s, d, ui, &proj);
    if s.show_gizmo && s.show_tool_gizmo && !is_modal(s) {
        widgets::draw_tool_gizmo(s, d, ui, &proj);
    }
    if s.show_overlays && s.show_text {
        overlay::text_info(s, d, ui, r, lay);
    }
    if s.show_toolbar {
        widgets::toolbar(s, d, ui, lay.toolbar, r);
    }
    if s.show_sidebar {
        sidebar::draw(s, d, ui, lay.sidebar);
    }
    if s.show_gizmo && s.show_nav_gizmo {
        widgets::nav_gizmo(s, d, ui, lay.nav, r);
    }
}

// ---------------------------------------------------------------- events

fn handle_event(s: &mut State, d: &mut Doc, ui: &mut Ui, inp: Input, r: Rect) {
    match inp {
        Input::Down { x, y, button: Button::Middle, mods } if r.contains(x, y) => {
            ui.input = None;
            s.last = (x, y);
            s.modal = if mods.shift {
                Modal::Pan
            } else if mods.ctrl {
                Modal::Zoom
            } else {
                Modal::Orbit
            };
        }
        Input::Wheel { x, y, steps, mods } if r.contains(x, y) => {
            ui.input = None;
            if mods.ctrl {
                pan_by(s, -(steps as f32) * 30.0, 0.0, r);
            } else if mods.shift {
                pan_by(s, 0.0, steps as f32 * 30.0, r);
            } else {
                zoom_by(s, 0.85f32.powi(steps));
            }
        }
        Input::Down { x, y, button: Button::Left, mods } if r.contains(x, y) => {
            ui.input = None;
            left_down(s, d, ui, x, y, mods, r);
        }
        Input::Down { x, y, button: Button::Right, mods } if r.contains(x, y) => {
            ui.input = None;
            let p = Proj::new(&s.cam, r);
            if mods.shift {
                d.cursor = surface_point(s, d, &p, x as f32, y as f32);
            } else {
                let (title, mut items) = if d.mode == Mode::Edit { menus::edit_context_menu(d) } else { ("Object Context Menu".to_string(), menus::object_context_menu(d)) };
                tag(&mut items, (x, y));
                ui.st.open_popup = Some(Popup::new(&title, items, x - 20, y - 10));
            }
        }
        Input::Key { key, mods, .. } if ui.hovered(r) => {
            if key_down(s, d, ui, key, mods, r) {
                ui.input = None;
            }
        }
        _ => {}
    }
}

fn left_down(s: &mut State, d: &mut Doc, ui: &mut Ui, x: i32, y: i32, mods: Mods, r: Rect) {
    let p = Proj::new(&s.cam, r);
    let (fx, fy) = (x as f32, y as f32);
    if s.show_gizmo && s.show_tool_gizmo {
        if let Some((mode, axis, plane)) = widgets::gizmo_hit(s, d, &p, fx, fy) {
            let space = d.orientation;
            if let Some(t) = transform::Modal::start(d, mode, &p, (fx, fy)) {
                let mut t = match axis {
                    Some(a) => t.with_axis(d, &p, a, plane, space),
                    None => t,
                };
                t.release_confirms = true;
                t.apply(d, &p);
                s.modal = Modal::Transform(Box::new(t));
                return;
            }
        }
    }
    let edit_mode = d.mode == Mode::Edit;
    match d.tool {
        Tool::Cursor => d.cursor = surface_point(s, d, &p, fx, fy),
        Tool::Annotate => {
            s.annotations.push(vec![p.at_depth(fx, fy, d.cursor)]);
            s.modal = Modal::Annotate;
        }
        Tool::Measure => {
            let a = surface_point(s, d, &p, fx, fy);
            s.measure = Some((a, a));
            s.modal = Modal::Measure;
        }
        Tool::AddCube => {
            let at = surface_point(s, d, &p, fx, fy);
            d.cursor = at;
            d.queue.push(Op::with("mesh.primitive_add", "cube"));
        }
        Tool::Extrude if edit_mode => extrude_move(s, d, (fx, fy), r),
        Tool::Inset if edit_mode => start_mesh_tool(s, d, MeshToolKind::Inset, (fx, fy), r),
        Tool::Bevel if edit_mode => start_mesh_tool(s, d, MeshToolKind::Bevel, (fx, fy), r),
        Tool::LoopCut if edit_mode => {
            s.modal = Modal::LoopCut { edge: None, cuts: 1, preview: Vec::new() };
            loopcut_hover(s, d, (fx, fy), r);
            loopcut_click(s, d);
        }
        Tool::Knife if edit_mode => s.modal = Modal::Knife { pts: vec![(fx, fy)] },
        Tool::Smooth if edit_mode => d.queue.push(Op::new("mesh.vertices_smooth")),
        Tool::PolyBuild | Tool::Spin | Tool::EdgeSlide | Tool::ShrinkFatten | Tool::Shear | Tool::Rip if edit_mode => {
            d.report = format!("The {} tool is not available yet", d.tool.label());
        }
        _ => s.modal = Modal::Press { x, y, shift: mods.shift, ctrl: mods.ctrl },
    }
    let _ = ui;
}

/// A left click without a drag: select what is under the pointer.
fn click_select(s: &mut State, d: &mut Doc, x: i32, y: i32, extend: bool, r: Rect) {
    let p = Proj::new(&s.cam, r);
    if d.mode == Mode::Edit {
        let xray = s.xray_on();
        let hit = edit_view(s, d, &p).and_then(|ev| ev.pick(d, x, y, xray));
        if hit.is_some() || !extend {
            edit::select_elem(d, hit, extend);
        }
        return;
    }
    ensure_cache(s, d);
    let hit = s.cache.as_ref().and_then(|(_, vp, _)| vp.pick(&s.cam, r.w as u32, r.h as u32, (x - r.x) as f32, (y - r.y) as f32));
    match (hit, extend) {
        (Some(i), true) => d.toggle_select(i),
        (Some(i), false) => d.select_only(Some(i)),
        (None, false) => d.select_only(None),
        (None, true) => {}
    }
}

fn box_select(s: &mut State, d: &mut Doc, b: Rect, mode: BoxMode, r: Rect) {
    let p = Proj::new(&s.cam, r);
    if d.mode == Mode::Edit {
        let xray = s.xray_on();
        let hits = edit_view(s, d, &p).map(|ev| ev.in_box(d, b, xray)).unwrap_or_default();
        edit::select_set(d, hits, mode);
        return;
    }
    ensure_world(s, d);
    let inside = |q: Vec3| p.to_screen(q).is_some_and(|(x, y)| b.contains(x as i32, y as i32));
    let mut hits: Vec<usize> = Vec::new();
    for i in 0..d.scene.objects.len() {
        if !d.scene.is_visible(i) {
            continue;
        }
        let mut hit = inside(d.world_location(i));
        if !hit {
            if let Some((_, objs)) = &s.world {
                if let Some(w) = objs.iter().find(|w| w.index == i) {
                    hit = w.points.iter().any(|&q| inside(q));
                }
            }
        }
        if hit {
            hits.push(i);
        }
    }
    let cur = d.scene.selected.clone();
    let sel: Vec<usize> = match mode {
        BoxMode::Set => hits.clone(),
        BoxMode::Extend => cur.iter().copied().chain(hits.iter().copied().filter(|h| !cur.contains(h))).collect(),
        BoxMode::Subtract => cur.iter().copied().filter(|c| !hits.contains(c)).collect(),
        BoxMode::Invert => cur.iter().copied().filter(|c| !hits.contains(c)).chain(hits.iter().copied().filter(|h| !cur.contains(h))).collect(),
        BoxMode::Intersect => cur.iter().copied().filter(|c| hits.contains(c)).collect(),
    };
    d.scene.selected = sel;
    if d.active().is_none_or(|a| !d.scene.selected.contains(&a)) {
        if let Some(&h) = hits.iter().find(|h| d.scene.selected.contains(h)) {
            d.scene.active_object = Some(h);
        }
    }
    d.touch();
}

fn popup_at(d: &mut Doc, ui: &Ui, title: &str, mut items: Vec<MenuItem>, r: Rect) {
    let (x, y) = ui.mouse();
    tag(&mut items, (r.x + 1, r.y + 1));
    d.menu = Some(Popup::new(title, items, x - 20, y - 10));
}

/// Keys over the view (not during a modal tool). Returns true when used.
fn key_down(s: &mut State, d: &mut Doc, ui: &mut Ui, key: Key, mods: Mods, r: Rect) -> bool {
    use Key as K;
    let edit_mode = d.mode == Mode::Edit;
    let m = mouse_in(ui, r);
    let q = |d: &mut Doc, id: &str, arg: &str| d.queue.push(Op::with(id, arg));
    match (key, mods) {
        // ---- navigation
        (K::Num(0), mm) if mm.ctrl => run_view_op(s, d, ui, "view3d.object_as_camera", "", r),
        (K::Num(0), _) => run_view_op(s, d, ui, "view3d.view_camera", "", r),
        (K::Num(1), mm) => set_axis_view(s, if mm.ctrl { "BACK" } else { "FRONT" }),
        (K::Num(3), mm) => set_axis_view(s, if mm.ctrl { "LEFT" } else { "RIGHT" }),
        (K::Num(7), mm) => set_axis_view(s, if mm.ctrl { "BOTTOM" } else { "TOP" }),
        (K::Num(5), _) => run_view_op(s, d, ui, "view3d.view_persportho", "", r),
        (K::Num(9), _) => run_view_op(s, d, ui, "view3d.view_orbit", "OPPOSITE", r),
        (K::Num(4), mm) if mm.ctrl => run_view_op(s, d, ui, "view3d.view_pan", "LEFT", r),
        (K::Num(6), mm) if mm.ctrl => run_view_op(s, d, ui, "view3d.view_pan", "RIGHT", r),
        (K::Num(8), mm) if mm.ctrl => run_view_op(s, d, ui, "view3d.view_pan", "UP", r),
        (K::Num(2), mm) if mm.ctrl => run_view_op(s, d, ui, "view3d.view_pan", "DOWN", r),
        (K::Num(4), _) => run_view_op(s, d, ui, "view3d.view_orbit", "LEFT", r),
        (K::Num(6), _) => run_view_op(s, d, ui, "view3d.view_orbit", "RIGHT", r),
        (K::Num(8), _) => run_view_op(s, d, ui, "view3d.view_orbit", "UP", r),
        (K::Num(2), _) => run_view_op(s, d, ui, "view3d.view_orbit", "DOWN", r),
        (K::NumPlus, mm) if mm.ctrl && edit_mode => q(d, "mesh.select_more", ""),
        (K::NumMinus, mm) if mm.ctrl && edit_mode => q(d, "mesh.select_less", ""),
        (K::NumPlus, _) => zoom_by(s, 0.85),
        (K::NumMinus, _) => zoom_by(s, 1.0 / 0.85),
        (K::NumDot, _) => frame_bounds(s, selected_bounds(d)),
        (K::Home, _) => frame_all(s, d),
        (K::NumSlash, _) => d.report = "view3d.localview is not available yet".into(),
        (K::Char('c'), Mods::SHIFT) => run_view_op(s, d, ui, "view3d.view_center_cursor", "", r),
        (K::Char('`'), _) => popup_at(d, ui, "View", menus::view_pie(), r),
        (K::Char('z'), Mods::NONE) => popup_at(d, ui, "Shading", menus::shading_pie(), r),
        (K::Char('z'), mm) if mm.alt && !mm.ctrl => s.xray = !s.xray,
        (K::Char('t'), Mods::NONE) => s.show_toolbar = !s.show_toolbar,
        (K::Char('n'), Mods::NONE) => s.show_sidebar = !s.show_sidebar,
        (K::Char('b'), Mods::NONE) => s.modal = Modal::BoxWait,
        (K::Tab, Mods::SHIFT) => d.snap = !d.snap,
        (K::Char('o'), Mods::NONE) => d.proportional = !d.proportional,
        (K::Char('.'), Mods::NONE) => popup_at(d, ui, "Pivot Point", menus::pivot_items(d), r),
        (K::Char(','), Mods::NONE) => popup_at(d, ui, "Transform Orientation", menus::orientation_items(d), r),
        (K::Char('s'), Mods::SHIFT) => popup_at(d, ui, "Snap", menus::snap_items(), r),
        (K::Char('a'), Mods::SHIFT) => popup_at(d, ui, "Add", menus::add_menu(d), r),
        (K::Tab, Mods::NONE) => q(d, "object.mode_set", "TOGGLE_EDIT"),
        (K::Tab, Mods::CTRL) => popup_at(d, ui, "Mode", menus::mode_items(), r),
        // ---- transforms (both modes)
        (K::Char('g'), Mods::NONE) => start_transform(s, d, TMode::Translate, m, r),
        (K::Char('r'), Mods::NONE) => start_transform(s, d, TMode::Rotate, m, r),
        (K::Char('s'), Mods::NONE) => start_transform(s, d, TMode::Resize, m, r),
        // ---- edit mode
        (K::Char(c @ '1'..='3'), Mods::NONE) if edit_mode => q(d, "mesh.select_mode", ["VERT", "EDGE", "FACE"][(c as u8 - b'1') as usize]),
        (K::Char('a'), Mods::NONE) if edit_mode => q(d, "mesh.select_all", "SELECT"),
        (K::Char('a'), mm) if edit_mode && mm.alt => q(d, "mesh.select_all", "DESELECT"),
        (K::Char('i'), Mods::CTRL) if edit_mode => q(d, "mesh.select_all", "INVERT"),
        (K::Char('l'), mm) if edit_mode && !mm.ctrl => {
            let p = Proj::new(&s.cam, r);
            let xray = s.xray_on();
            let hit = edit_view(s, d, &p).and_then(|ev| ev.pick(d, m.0 as i32, m.1 as i32, xray));
            if let Some(e) = hit {
                edit::select_linked_pick(d, e, mm.shift);
            }
        }
        (K::Char('l'), Mods::CTRL) if edit_mode => q(d, "mesh.select_linked", ""),
        (K::Char('e'), Mods::NONE) if edit_mode => extrude_move(s, d, m, r),
        (K::Char('i'), Mods::NONE) if edit_mode => start_mesh_tool(s, d, MeshToolKind::Inset, m, r),
        (K::Char('b'), Mods::CTRL) if edit_mode => start_mesh_tool(s, d, MeshToolKind::Bevel, m, r),
        (K::Char('r'), Mods::CTRL) if edit_mode => run_view_op(s, d, ui, "mesh.loopcut_slide", "", r),
        (K::Char('k'), Mods::NONE) if edit_mode => run_view_op(s, d, ui, "mesh.knife_tool", "", r),
        (K::Char('f'), Mods::NONE) if edit_mode => q(d, "mesh.edge_face_add", ""),
        (K::Char('m'), Mods::NONE) if edit_mode => popup_at(d, ui, "Merge", menus::merge_items(), r),
        (K::Char('x'), Mods::NONE) | (K::Delete, _) if edit_mode => popup_at(d, ui, "Delete", menus::delete_items(), r),
        (K::Char('n'), Mods::CTRL) if edit_mode => q(d, "mesh.normals_make_consistent", ""),
        (K::Char('n'), Mods::CTRL_SHIFT) if edit_mode => q(d, "mesh.normals_make_consistent", "INSIDE"),
        (K::Char('n'), Mods::SHIFT) if edit_mode => q(d, "mesh.normals_make_consistent", ""),
        (K::Char('d'), Mods::SHIFT) if edit_mode => run_view_op(s, d, ui, "mesh.duplicate_move", "", r),
        (K::Char('p'), Mods::NONE) if edit_mode => popup_at(d, ui, "Separate", menus::separate_items(), r),
        (K::Char('h'), Mods::NONE) if edit_mode => q(d, "mesh.hide", ""),
        (K::Char('h'), Mods::SHIFT) if edit_mode => q(d, "mesh.hide", "UNSELECTED"),
        (K::Char('h'), mm) if edit_mode && mm.alt => q(d, "mesh.reveal", ""),
        (K::Char('t'), Mods::CTRL) if edit_mode => q(d, "mesh.quads_convert_to_tris", ""),
        (K::Char('v'), Mods::CTRL) if edit_mode => popup_at(d, ui, "Vertex", menus::vertex_menu(d), r),
        (K::Char('e'), Mods::CTRL) if edit_mode => popup_at(d, ui, "Edge", menus::edge_menu(d), r),
        (K::Char('f'), Mods::CTRL) if edit_mode => popup_at(d, ui, "Face", menus::face_menu(d), r),
        (K::Char('u'), Mods::NONE) if edit_mode => popup_at(d, ui, "UV", menus::uv_menu(d), r),
        // ---- object mode
        (K::Char('a'), Mods::NONE) => q(d, "object.select_all", "SELECT"),
        (K::Char('a'), mm) if mm.alt => q(d, "object.select_all", "DESELECT"),
        (K::Char('i'), Mods::CTRL) => q(d, "object.select_all", "INVERT"),
        (K::Char('g'), mm) if mm.alt => q(d, "object.location_clear", ""),
        (K::Char('r'), mm) if mm.alt => q(d, "object.rotation_clear", ""),
        (K::Char('s'), mm) if mm.alt => q(d, "object.scale_clear", ""),
        (K::Char('d'), Mods::SHIFT) => run_view_op(s, d, ui, "object.duplicate_move", "", r),
        (K::Char('d'), mm) if mm.alt => run_view_op(s, d, ui, "object.duplicate_move_linked", "", r),
        (K::Char('x'), Mods::NONE) => {
            if !d.scene.selected.is_empty() {
                popup_at(d, ui, "OK?", menus::delete_confirm(), r);
            }
        }
        (K::Delete, _) => q(d, "object.delete", ""),
        (K::Char('h'), Mods::NONE) => q(d, "object.hide_view_set", ""),
        (K::Char('h'), Mods::SHIFT) => q(d, "object.hide_view_set", "UNSELECTED"),
        (K::Char('h'), mm) if mm.alt => q(d, "object.hide_view_clear", ""),
        (K::Char('m'), Mods::NONE) => {
            if !d.scene.selected.is_empty() {
                popup_at(d, ui, "Move to Collection", menus::collection_items(d), r);
            }
        }
        (K::Char('a'), Mods::CTRL) => popup_at(d, ui, "Apply", menus::apply_items(), r),
        (K::Char('p'), Mods::CTRL) => popup_at(d, ui, "Set Parent To", menus::parent_items(), r),
        (K::Char('p'), mm) if mm.alt => popup_at(d, ui, "Clear Parent", menus::clear_parent_items(), r),
        (K::Char('j'), Mods::CTRL) => q(d, "object.join", ""),
        (K::Char('i'), Mods::NONE) => q(d, "anim.keyframe_insert", ""),
        (K::Char('o'), mm) if mm.alt => q(d, "object.origin_clear", ""),
        (K::F(2), _) => d.report = "Rename is not available yet".into(),
        (K::F(9), _) => d.report = "screen.redo_last is not available yet".into(),
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------- modal

fn loopcut_hover(s: &mut State, d: &Doc, m: (f32, f32), r: Rect) {
    let p = Proj::new(&s.cam, r);
    let xray = s.xray_on();
    let edge = edit_view(s, d, &p).and_then(|ev| ev.nearest_edge(d, m.0, m.1, xray, 40.0));
    let Modal::LoopCut { edge: cur, cuts, preview } = &mut s.modal else { return };
    if *cur == edge && !preview.is_empty() {
        return;
    }
    *cur = edge;
    preview.clear();
    let (Some(e), Some((oi, mi))) = (edge, edit::active_mesh(d)) else { return };
    let (cut, new) = edit::loop_cut_preview(&d.scene.meshes[mi], e, *cuts);
    let w = d.scene.world_matrix(oi);
    let set: std::collections::HashSet<u32> = new.iter().copied().collect();
    for ed in cut.edges() {
        if set.contains(&ed[0]) && set.contains(&ed[1]) {
            preview.push((w.transform_point(cut.positions[ed[0] as usize]), w.transform_point(cut.positions[ed[1] as usize])));
        }
    }
}

fn loopcut_click(s: &mut State, d: &mut Doc) {
    if let Modal::LoopCut { edge: Some(e), cuts, .. } = s.modal {
        edit::loop_cut(d, e, cuts);
        s.modal = Modal::None;
    }
}

fn knife_apply(s: &mut State, d: &mut Doc, pts: &[(f32, f32)], r: Rect) {
    let p = Proj::new(&s.cam, r);
    let center = d.selection_center().or_else(|| d.active().map(|a| d.world_location(a))).unwrap_or(Vec3::ZERO);
    for w in pts.windows(2) {
        let a = p.at_depth(w[0].0, w[0].1, center);
        let b = p.at_depth(w[1].0, w[1].1, center);
        let (_, dir) = p.ray((w[0].0 + w[1].0) * 0.5, (w[0].1 + w[1].1) * 0.5);
        let n = (b - a).cross(dir).normalize();
        if n != Vec3::ZERO {
            edit::bisect(d, a, n);
        }
    }
}

fn mesh_tool_apply(t: &mut MeshTool, d: &mut Doc, p: &Proj, mouse: (f32, f32)) {
    let c = p.to_screen(t.center).unwrap_or((p.cx, p.cy));
    let d0 = (t.start.0 - c.0).hypot(t.start.1 - c.1);
    let d1 = (mouse.0 - c.0).hypot(mouse.1 - c.1);
    let k = p.world_per_pixel(t.center);
    t.value = match crate::ui::eval_number(&t.numeric).filter(|_| !t.numeric.is_empty()) {
        Some(v) => v.max(0.0),
        None => ((d1 - d0) * k).max(0.0),
    };
    let new = match t.kind {
        MeshToolKind::Inset => edit::inset_mesh(&t.orig, &t.faces, t.value, false),
        MeshToolKind::Bevel => edit::bevel_mesh(&t.orig, &t.edges, t.value, t.segments).0,
    };
    if let Some(m) = d.scene.meshes.get_mut(t.mi) {
        *m = new;
    }
    d.touch();
}

fn mesh_tool_finish(t: &mut MeshTool, d: &mut Doc, p: &Proj, mouse: (f32, f32), confirm: bool) {
    d.scene.meshes[t.mi] = t.orig.clone();
    if confirm {
        d.push_undo(if t.kind == MeshToolKind::Inset { "Inset Faces" } else { "Bevel" });
        mesh_tool_apply(t, d, p, mouse);
        if t.kind == MeshToolKind::Bevel {
            let (_, faces) = edit::bevel_mesh(&t.orig, &t.edges, t.value, t.segments);
            if !faces.is_empty() {
                edit::select_faces(d, &faces);
            }
        } else {
            edit::select_faces(d, &t.faces);
        }
        edit::flush(d);
    }
    d.touch();
}

fn handle_modal(s: &mut State, d: &mut Doc, ui: &mut Ui, inp: Input, r: Rect) {
    let p = Proj::new(&s.cam, r);
    let mouse = ui.mouse();
    let mf = (mouse.0 as f32, mouse.1 as f32);
    // Every event belongs to the modal tool while it runs.
    ui.input = None;
    let modal = core::mem::replace(&mut s.modal, Modal::None);
    s.modal = match modal {
        Modal::None => Modal::None,
        Modal::Orbit | Modal::Pan | Modal::Zoom => match inp {
            Input::Move { x, y } => {
                let (dx, dy) = ((x - s.last.0) as f32, (y - s.last.1) as f32);
                s.last = (x, y);
                match modal {
                    Modal::Orbit => orbit_by(s, -dx * 0.008, dy * 0.008),
                    Modal::Pan => pan_by(s, dx, dy, r),
                    _ => zoom_by(s, 1.006f32.powf(dy)),
                }
                modal
            }
            Input::Up { .. } | Input::Key { key: Key::Esc, .. } => Modal::None,
            _ => modal,
        },
        Modal::Press { x, y, shift, ctrl } => match inp {
            Input::Move { x: mx, y: my } if (mx - x).abs() + (my - y).abs() > 4 => {
                let mode = if shift {
                    BoxMode::Extend
                } else if ctrl {
                    BoxMode::Subtract
                } else {
                    d.box_mode
                };
                Modal::Box { x0: x, y0: y, x1: mx, y1: my, mode }
            }
            Input::Up { button: Button::Left, .. } => {
                click_select(s, d, x, y, shift, r);
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } => Modal::None,
            _ => modal,
        },
        Modal::Box { x0, y0, mode, .. } => match inp {
            Input::Move { x, y } => Modal::Box { x0, y0, x1: x, y1: y, mode },
            Input::Up { x, y, .. } => {
                let b = Rect::new(x0.min(x), y0.min(y), (x - x0).abs() + 1, (y - y0).abs() + 1);
                box_select(s, d, b, mode, r);
                Modal::None
            }
            Input::Key { key: Key::Esc, .. } | Input::Down { button: Button::Right, .. } => Modal::None,
            _ => modal,
        },
        Modal::BoxWait => match inp {
            Input::Down { x, y, button: Button::Left, mods } => Modal::Box { x0: x, y0: y, x1: x, y1: y, mode: if mods.ctrl { BoxMode::Subtract } else { BoxMode::Extend } },
            Input::Down { x, y, button: Button::Middle, .. } => Modal::Box { x0: x, y0: y, x1: x, y1: y, mode: BoxMode::Subtract },
            Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => Modal::None,
            _ => modal,
        },
        Modal::Transform(mut m) => {
            let mut done: Option<bool> = None;
            match inp {
                Input::Move { x, y } => {
                    m.mouse_to(&p, (x as f32, y as f32));
                    m.apply(d, &p);
                }
                Input::Down { button: Button::Left, .. } => done = Some(true),
                Input::Down { button: Button::Right, .. } => done = Some(false),
                Input::Up { button: Button::Left, .. } if m.release_confirms => done = Some(true),
                Input::Wheel { steps, .. } => {
                    if d.proportional && m.is_edit() {
                        d.proportional_size = (d.proportional_size * 1.1f32.powi(-steps)).max(1e-3);
                        m.update_weights(d);
                        m.apply(d, &p);
                    }
                }
                Input::Key { key, mods, text } => match m.key(d, &p, key, mods, text) {
                    KeyResult::Confirm => done = Some(true),
                    KeyResult::Cancel => done = Some(false),
                    KeyResult::Continue => m.apply(d, &p),
                    KeyResult::Ignored => {}
                },
                _ => {}
            }
            match done {
                Some(true) => {
                    m.confirm(d, &p);
                    Modal::None
                }
                Some(false) => {
                    m.restore(d);
                    Modal::None
                }
                None => Modal::Transform(m),
            }
        }
        Modal::MeshTool(mut t) => {
            let mut done: Option<bool> = None;
            match inp {
                Input::Move { x, y } => mesh_tool_apply(&mut t, d, &p, (x as f32, y as f32)),
                Input::Down { button: Button::Left, .. } => done = Some(true),
                Input::Down { button: Button::Right, .. } => done = Some(false),
                Input::Wheel { steps, .. } => {
                    t.segments = (t.segments as i32 + steps).clamp(1, 100) as u32;
                    mesh_tool_apply(&mut t, d, &p, mf);
                }
                Input::Key { key, .. } => match key {
                    Key::Enter | Key::NumEnter | Key::Space => done = Some(true),
                    Key::Esc => done = Some(false),
                    Key::PageUp | Key::NumPlus => {
                        t.segments = (t.segments + 1).min(100);
                        mesh_tool_apply(&mut t, d, &p, mf);
                    }
                    Key::PageDown => {
                        t.segments = t.segments.saturating_sub(1).max(1);
                        mesh_tool_apply(&mut t, d, &p, mf);
                    }
                    Key::Backspace => {
                        t.numeric.pop();
                        mesh_tool_apply(&mut t, d, &p, mf);
                    }
                    Key::Char(c) if c.is_ascii_digit() || c == '.' => {
                        t.numeric.push(c);
                        mesh_tool_apply(&mut t, d, &p, mf);
                    }
                    Key::Num(n) => {
                        t.numeric.push((b'0' + n) as char);
                        mesh_tool_apply(&mut t, d, &p, mf);
                    }
                    Key::NumDot => {
                        t.numeric.push('.');
                        mesh_tool_apply(&mut t, d, &p, mf);
                    }
                    _ => {}
                },
                _ => {}
            }
            match done {
                Some(c) => {
                    mesh_tool_finish(&mut t, d, &p, mf, c);
                    Modal::None
                }
                None => Modal::MeshTool(t),
            }
        }
        Modal::LoopCut { edge, mut cuts, preview } => {
            let mut next = Some(Modal::LoopCut { edge, cuts, preview });
            match inp {
                Input::Move { x, y } => {
                    s.modal = next.take().unwrap_or(Modal::None);
                    loopcut_hover(s, d, (x as f32, y as f32), r);
                    return;
                }
                Input::Down { button: Button::Left, .. } => {
                    s.modal = next.take().unwrap_or(Modal::None);
                    loopcut_click(s, d);
                    return;
                }
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => next = None,
                Input::Wheel { steps, .. } => cuts = (cuts as i32 + steps).clamp(1, 64) as u32,
                Input::Key { key: Key::PageUp | Key::NumPlus, .. } => cuts = (cuts + 1).min(64),
                Input::Key { key: Key::PageDown | Key::NumMinus, .. } => cuts = cuts.saturating_sub(1).max(1),
                _ => {}
            }
            if let Some(Modal::LoopCut { edge, cuts: old, .. }) = next {
                if old != cuts {
                    s.modal = Modal::LoopCut { edge: None, cuts, preview: Vec::new() };
                    let _ = edge;
                    loopcut_hover(s, d, mf, r);
                    return;
                }
                s.modal = Modal::LoopCut { edge, cuts, preview: Vec::new() };
                loopcut_hover(s, d, mf, r);
                return;
            }
            Modal::None
        }
        Modal::Knife { mut pts } => match inp {
            Input::Down { x, y, button: Button::Left, .. } => {
                pts.push((x as f32, y as f32));
                Modal::Knife { pts }
            }
            Input::Key { key: Key::Enter | Key::NumEnter | Key::Space, .. } => {
                knife_apply(s, d, &pts, r);
                Modal::None
            }
            Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => Modal::None,
            _ => Modal::Knife { pts },
        },
        Modal::Annotate => match inp {
            Input::Move { x, y } => {
                let q = p.at_depth(x as f32, y as f32, d.cursor);
                if let Some(st) = s.annotations.last_mut() {
                    st.push(q);
                }
                Modal::Annotate
            }
            Input::Up { .. } => Modal::None,
            _ => Modal::Annotate,
        },
        Modal::Measure => match inp {
            Input::Move { x, y } => {
                let b = surface_point(s, d, &p, x as f32, y as f32);
                if let Some(m) = &mut s.measure {
                    m.1 = b;
                }
                Modal::Measure
            }
            Input::Up { .. } => Modal::None,
            _ => Modal::Measure,
        },
    };
}

/// Keys that reach the area outside of a frame's widgets (none: the body
/// handles the viewport's keys while the pointer is over it).
pub fn hotkey(_s: &mut State, _d: &mut Doc, _input: &Input, _r: Rect) -> bool {
    false
}

