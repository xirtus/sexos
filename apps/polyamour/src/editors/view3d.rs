// SPDX-License-Identifier: GPL-3.0-only
//! The 3D Viewport: Blender's header (mode, View/Select/Add/Object menus), the
//! Solid-mode CPU viewport from sex-3d, middle-mouse orbit, Shift+middle pan,
//! wheel zoom, click to select.
use super::header_menus;
use crate::doc::{Doc, Mode};
use crate::draw::Rect;
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{MenuItem, Op, Ui};
use sex_3d::render::viewport::{ViewCamera, ViewportScene, ViewportSettings};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    None,
    Orbit,
    Pan,
}

pub struct State {
    pub cam: ViewCamera,
    /// Cached viewport geometry and the document generation it was built from.
    cache: Option<(u64, ViewportScene)>,
    buf: Vec<u8>,
    pub drag: Drag,
    last: (i32, i32),
    pub wireframe: bool,
}

impl Default for State {
    fn default() -> Self {
        State { cam: ViewCamera::default(), cache: None, buf: Vec::new(), drag: Drag::None, last: (0, 0), wireframe: false }
    }
}

pub fn is_modal(s: &State) -> bool {
    s.drag != Drag::None
}

fn add_menu(_d: &Doc) -> Vec<MenuItem> {
    let p = |l: &str, a: &str| MenuItem::op(l, "", Op::with("mesh.primitive_add", a));
    vec![
        MenuItem::Sub(
            "Mesh".into(),
            vec![p("Plane", "plane"), p("Cube", "cube"), p("Circle", "circle"), p("UV Sphere", "uv_sphere"), p("Ico Sphere", "ico_sphere"), p("Cylinder", "cylinder"), p("Cone", "cone"), p("Torus", "torus"), MenuItem::Separator, p("Grid", "grid")],
        ),
        MenuItem::op("Empty", "", Op::new("object.empty_add")),
        MenuItem::Separator,
        MenuItem::op("Camera", "", Op::new("object.camera_add")),
        MenuItem::Sub(
            "Light".into(),
            vec![
                MenuItem::op("Point", "", Op::with("object.light_add", "POINT")),
                MenuItem::op("Sun", "", Op::with("object.light_add", "SUN")),
                MenuItem::op("Spot", "", Op::with("object.light_add", "SPOT")),
                MenuItem::op("Area", "", Op::with("object.light_add", "AREA")),
            ],
        ),
    ]
}

fn object_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Duplicate Objects", "Shift D", Op::new("object.duplicate")),
        MenuItem::op("Delete", "X", Op::new("object.delete")),
        MenuItem::Separator,
        MenuItem::op("Shade Smooth", "", Op::new("object.shade_smooth")),
        MenuItem::op("Shade Flat", "", Op::new("object.shade_flat")),
        MenuItem::Separator,
        MenuItem::Sub(
            "Show/Hide".into(),
            vec![MenuItem::op("Show Hidden Objects", "Alt H", Op::new("object.hide_view_clear")), MenuItem::op("Hide Selected", "H", Op::new("object.hide_view_set"))],
        ),
    ]
}

fn select_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("All", "A", Op::with("object.select_all", "SELECT")),
        MenuItem::op("None", "Alt A", Op::with("object.select_all", "DESELECT")),
        MenuItem::op("Invert", "Ctrl I", Op::with("object.select_all", "INVERT")),
    ]
}

fn view_menu(_d: &Doc) -> Vec<MenuItem> {
    vec![
        MenuItem::op("Frame All", "Home", Op::new("view3d.view_all")),
        MenuItem::op("Frame Selected", "Numpad .", Op::new("view3d.view_selected")),
        MenuItem::Separator,
        MenuItem::op("Perspective/Orthographic", "Numpad 5", Op::new("view3d.view_persportho")),
    ]
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let mr = Rect::new(hr.x, hr.y + 3, 112, hr.h - 6);
    if ui.dropdown(mr, d.mode.label()) {
        let items = Mode::ALL
            .iter()
            .map(|m| {
                let arg = match m {
                    Mode::Object => "OBJECT",
                    Mode::Edit => "EDIT",
                    Mode::Sculpt => "SCULPT",
                    Mode::VertexPaint => "VERTEX_PAINT",
                    Mode::WeightPaint => "WEIGHT_PAINT",
                    Mode::TexturePaint => "TEXTURE_PAINT",
                };
                MenuItem::op(m.label(), "", Op::with("object.mode_set", arg))
            })
            .collect();
        ui.open_menu(mr, "", items);
    }
    let menus: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 4] = [("View", &view_menu), ("Select", &select_menu), ("Add", &add_menu), ("Object", &object_menu)];
    header_menus(ui, hr, mr.right() + 8, &menus, d);
    // Viewport shading buttons at the right: wireframe / solid.
    let mut shade = if s.wireframe { 0 } else { 1 };
    let sr = Rect::new(hr.right() - 70, hr.y + 3, 64, hr.h - 6);
    if ui.segmented(sr, &["W", "S"], &mut shade) {
        s.wireframe = shade == 0;
    }
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if r.is_empty() {
        return;
    }
    let stale = s.cache.as_ref().map_or(true, |c| c.0 != d.generation);
    if stale {
        s.cache = Some((d.generation, ViewportScene::new(&d.scene)));
    }
    let settings = ViewportSettings { width: r.w as u32, height: r.h as u32, wireframe: s.wireframe, grid: true, backface_culling: false, threads: 0 };
    if let Some((_, vp)) = &s.cache {
        vp.draw_into(&s.cam, &settings, &mut s.buf);
        ui.p.blit_rgba(r, &s.buf, r.w as usize, r.h as usize);
    }
    // Mode name and object at the top-left, as Blender's text overlay does.
    let title = format!("User {}", if s.cam.ortho { "Ortho" } else { "Perspective" });
    ui.p.text(r.x + 10, r.y + 6, &title, t::TEXT);
    if let Some(a) = d.active() {
        let line = format!("(1) Collection | {}", d.scene.objects[a].name);
        ui.p.text(r.x + 10, r.y + 6 + ui.p.line_height(), &line, t::TEXT);
    }
    nav_gizmo(s, ui, r);

    match ui.input {
        Some(Input::Down { x, y, button: Button::Middle, mods }) if r.contains(x, y) => {
            ui.input = None;
            s.drag = if mods.shift { Drag::Pan } else { Drag::Orbit };
            s.last = (x, y);
        }
        Some(Input::Move { x, y }) if s.drag != Drag::None => {
            ui.input = None;
            let (dx, dy) = ((x - s.last.0) as f32, (y - s.last.1) as f32);
            s.last = (x, y);
            match s.drag {
                Drag::Orbit => {
                    s.cam.yaw -= dx * 0.008;
                    s.cam.pitch = (s.cam.pitch + dy * 0.008).clamp(-1.55, 1.55);
                }
                Drag::Pan => {
                    let k = s.cam.distance * 2.0 * (s.cam.fov_y * 0.5).tan() / r.h.max(1) as f32;
                    s.cam.target = s.cam.target - s.cam.right() * (dx * k) + s.cam.up() * (dy * k);
                }
                Drag::None => {}
            }
        }
        Some(Input::Up { button: Button::Middle, .. }) if s.drag != Drag::None => {
            ui.input = None;
            s.drag = Drag::None;
        }
        Some(Input::Wheel { x, y, steps, .. }) if r.contains(x, y) => {
            ui.input = None;
            s.cam.distance = (s.cam.distance * 0.85f32.powi(steps)).clamp(0.01, 1e5);
        }
        Some(Input::Down { x, y, button: Button::Left, mods }) if r.contains(x, y) => {
            ui.input = None;
            let hit = s.cache.as_ref().and_then(|(_, vp)| vp.pick(&s.cam, r.w as u32, r.h as u32, (x - r.x) as f32, (y - r.y) as f32));
            match (hit, mods.shift) {
                (Some(i), true) => d.toggle_select(i),
                (Some(i), false) => d.select_only(Some(i)),
                (None, false) => d.select_only(None),
                (None, true) => {}
            }
            if let Some((_, vp)) = &mut s.cache {
                vp.sync_selection(&d.scene);
            }
        }
        _ => {}
    }
}

/// Blender's navigation gizmo: the X/Y/Z axis balls at the top-right.
fn nav_gizmo(s: &State, ui: &mut Ui, r: Rect) {
    let (cx, cy, rad) = ((r.right() - 50) as f32, (r.y + 50) as f32, 32.0);
    let right = s.cam.right();
    let up = s.cam.up();
    let back = s.cam.back();
    let mut axes = [(sex_3d::Vec3::X, t::AXIS_X, "X"), (sex_3d::Vec3::Y, t::AXIS_Y, "Y"), (sex_3d::Vec3::Z, t::AXIS_Z, "Z")];
    // Far axes first so near ones draw over them.
    axes.sort_by(|a, b| a.0.dot(back).partial_cmp(&b.0.dot(back)).unwrap_or(core::cmp::Ordering::Equal));
    for (v, c, l) in axes {
        let (px, py) = (cx + v.dot(right) * rad, cy - v.dot(up) * rad);
        ui.p.line(cx, cy, px, py, c);
        ui.p.circle(px, py, 8.0, c);
        let w = ui.p.text_width(l);
        ui.p.text(px as i32 - w / 2, py as i32 - ui.p.line_height() / 2, l, 0x000000);
        let (nx, ny) = (cx - v.dot(right) * rad, cy + v.dot(up) * rad);
        ui.p.circle(nx, ny, 6.0, crate::draw_mix(c, 0x3d3d3d, 150));
    }
}

pub fn hotkey(s: &mut State, d: &mut Doc, input: &Input, r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *input else { return false };
    use Key as K;
    let op = |id: &str| Some(Op::new(id));
    let run = match (key, mods) {
        (K::Char('a'), Mods::NONE) => Some(Op::with("object.select_all", "SELECT")),
        (K::Char('a'), Mods::ALT) => Some(Op::with("object.select_all", "DESELECT")),
        (K::Char('i'), Mods::CTRL) => Some(Op::with("object.select_all", "INVERT")),
        (K::Char('x'), Mods::NONE) | (K::Delete, Mods::NONE) => op("object.delete"),
        (K::Char('d'), Mods::SHIFT) => op("object.duplicate"),
        (K::Char('h'), Mods::NONE) => op("object.hide_view_set"),
        (K::Char('h'), Mods::ALT) => op("object.hide_view_clear"),
        (K::Tab, Mods::NONE) => Some(Op::with("object.mode_set", "TOGGLE_EDIT")),
        (K::Char('a'), Mods::SHIFT) => {
            let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
            d.queue.push(Op::with("wm.call_menu", &format!("ADD:{x}:{y}")));
            return true;
        }
        _ => None,
    };
    if let Some(op) = run {
        d.queue.push(op);
        return true;
    }
    match (key, mods) {
        (K::Home, _) => {
            s.cam.frame(d.scene.bounds());
            true
        }
        (K::Num(5), _) => {
            s.cam.ortho = !s.cam.ortho;
            true
        }
        (K::Num(1), m) => set_view(s, if m.ctrl { core::f32::consts::FRAC_PI_2 } else { -core::f32::consts::FRAC_PI_2 }, 0.0),
        (K::Num(3), m) => set_view(s, if m.ctrl { core::f32::consts::PI } else { 0.0 }, 0.0),
        (K::Num(7), m) => set_view(s, -core::f32::consts::FRAC_PI_2, if m.ctrl { -1.5707 } else { 1.5707 }),
        (K::Num(4), _) => {
            s.cam.yaw += 15f32.to_radians();
            true
        }
        (K::Num(6), _) => {
            s.cam.yaw -= 15f32.to_radians();
            true
        }
        (K::Num(8), _) => {
            s.cam.pitch = (s.cam.pitch + 15f32.to_radians()).min(1.5707);
            true
        }
        (K::Num(2), _) => {
            s.cam.pitch = (s.cam.pitch - 15f32.to_radians()).max(-1.5707);
            true
        }
        (K::NumPlus, _) => {
            s.cam.distance *= 0.85;
            true
        }
        (K::NumMinus, _) => {
            s.cam.distance /= 0.85;
            true
        }
        _ => false,
    }
}

fn set_view(s: &mut State, yaw: f32, pitch: f32) -> bool {
    s.cam.yaw = yaw;
    s.cam.pitch = pitch;
    true
}
