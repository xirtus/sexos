// SPDX-License-Identifier: GPL-3.0-only
//! Blender's node editors on one canvas: the Shader Editor (the active
//! object's active material as Blender's default tree, a Principled BSDF linked
//! to a Material Output, or the World's Background node), the Compositor
//! (Render Layers to Composite and Viewer) and the Geometry Node Editor (Group
//! Input to Group Output). Shader node fields are bound to the material, so
//! editing Roughness on the node changes the material; the compositor and
//! geometry trees are shown but do not run yet.
//!
//! Canvas keys as in Blender: wheel zooms at the pointer, middle-drag or
//! Ctrl/Shift+wheel pans, Home frames all, click selects (Shift extends), drag
//! moves nodes, drag on empty space box-selects, G moves, A selects all.
use super::header_menus;
use crate::anim::{self, ReportKind};
use crate::doc::Doc;
use crate::draw::{Align, Painter, Rect};
use crate::editors::EditorKind;
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{eval_number, id, trim_number, MenuItem, Op, Ui};
use sex_3d::render::srgb_encode;
use sex_3d::scene::Material;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sock {
    Float,
    Vector,
    Color,
    Shader,
    Geometry,
    /// The empty "extend" socket of group nodes.
    Virtual,
}

/// Blender's socket colours by type.
pub fn sock_color(s: Sock) -> u32 {
    match s {
        Sock::Float => 0xa1a1a1,
        Sock::Vector => 0x6363c7,
        Sock::Color => 0xc7c729,
        Sock::Shader => 0x63c763,
        Sock::Geometry => 0x00d6a3,
        Sock::Virtual => 0x3b3b3b,
    }
}

/// What a field edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bind {
    None,
    BaseColor,
    Metallic,
    Roughness,
    Ior,
    Alpha,
    TransmissionWeight,
    EmissionColor,
    EmissionStrength,
    WorldColor,
    WorldStrength,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Field {
    /// Socket label only.
    Label,
    /// 0..1 slider with a fill.
    Factor,
    /// Unbounded number.
    Number,
    Color,
    /// One channel (0 = R) of the colour above, shown while it is open.
    Channel(u8),
}

#[derive(Clone, Debug, PartialEq)]
enum RowDef {
    Output(&'static str, Sock),
    /// Input socket with an optional field and a fixed value for unbound fields.
    Input(&'static str, Sock, Field, Bind, f32),
    Dropdown(&'static str),
    Check(&'static str, bool),
    /// A collapsible panel (Principled's Subsurface, Specular, ...).
    Panel(&'static str, usize),
}

struct NodeDef {
    title: &'static str,
    color: u32,
    width: f32,
    rows: Vec<RowDef>,
}

const HEADER_COL_SHADER: u32 = 0x2b652b;
const HEADER_COL_OUTPUT: u32 = 0x6a1e29;
const HEADER_COL_INPUT: u32 = 0x83314a;
const HEADER_COL_GROUP: u32 = 0x3b3b3b;
const NODE_BODY: u32 = 0x303030;
const ROW: f32 = 22.0;
const HEAD: f32 = 24.0;

const PANELS: [&str; 7] = ["Subsurface", "Specular", "Transmission", "Coat", "Sheen", "Emission", "Thin Film"];

fn principled(open: &[bool; 8]) -> NodeDef {
    use RowDef::*;
    let mut rows = vec![
        Output("BSDF", Sock::Shader),
        Dropdown("Multiscatter GGX"),
        Dropdown("Random Walk"),
        Input("Base Color", Sock::Color, Field::Color, Bind::BaseColor, 0.0),
        Input("Metallic", Sock::Float, Field::Factor, Bind::Metallic, 0.0),
        Input("Roughness", Sock::Float, Field::Factor, Bind::Roughness, 0.0),
        Input("IOR", Sock::Float, Field::Number, Bind::Ior, 0.0),
        Input("Alpha", Sock::Float, Field::Factor, Bind::Alpha, 0.0),
        Input("Normal", Sock::Vector, Field::Label, Bind::None, 0.0),
    ];
    for (i, p) in PANELS.iter().enumerate() {
        rows.push(Panel(p, i));
        if !open[i] {
            continue;
        }
        let extra: Vec<RowDef> = match i {
            0 => vec![Input("Weight", Sock::Float, Field::Factor, Bind::None, 0.0), Input("Radius", Sock::Vector, Field::Label, Bind::None, 0.0), Input("Scale", Sock::Float, Field::Number, Bind::None, 0.05)],
            1 => vec![Input("IOR Level", Sock::Float, Field::Factor, Bind::None, 0.5), Input("Tint", Sock::Color, Field::Label, Bind::None, 0.0), Input("Anisotropic", Sock::Float, Field::Factor, Bind::None, 0.0)],
            2 => vec![Input("Weight", Sock::Float, Field::Factor, Bind::TransmissionWeight, 0.0)],
            3 => vec![Input("Weight", Sock::Float, Field::Factor, Bind::None, 0.0), Input("Roughness", Sock::Float, Field::Factor, Bind::None, 0.03), Input("IOR", Sock::Float, Field::Number, Bind::None, 1.5)],
            4 => vec![Input("Weight", Sock::Float, Field::Factor, Bind::None, 0.0), Input("Roughness", Sock::Float, Field::Factor, Bind::None, 0.5)],
            5 => vec![Input("Color", Sock::Color, Field::Color, Bind::EmissionColor, 0.0), Input("Strength", Sock::Float, Field::Number, Bind::EmissionStrength, 0.0)],
            _ => vec![Input("Thickness", Sock::Float, Field::Number, Bind::None, 0.0), Input("IOR", Sock::Float, Field::Number, Bind::None, 1.33)],
        };
        rows.extend(extra);
    }
    NodeDef { title: "Principled BSDF", color: HEADER_COL_SHADER, width: 240.0, rows }
}

/// A tree: nodes (definition, default position) and links (from node, output
/// name, to node, input name).
struct Tree {
    nodes: Vec<(NodeDef, (f32, f32))>,
    links: Vec<(usize, &'static str, usize, &'static str)>,
}

fn tree(kind: EditorKind, world: bool, open: &[bool; 8], color_open: Option<Bind>) -> Tree {
    let mut t = base_tree(kind, world, open);
    if let Some(b) = color_open {
        for (def, _) in &mut t.nodes {
            if let Some(i) = def.rows.iter().position(|r| matches!(r, RowDef::Input(_, _, Field::Color, bb, _) if *bb == b)) {
                for (k, ch) in ["R", "G", "B"].iter().enumerate() {
                    def.rows.insert(i + 1 + k, RowDef::Input(ch, Sock::Virtual, Field::Channel(k as u8), b, 0.0));
                }
            }
        }
    }
    t
}

fn base_tree(kind: EditorKind, world: bool, open: &[bool; 8]) -> Tree {
    use RowDef::*;
    match kind {
        EditorKind::CompositorEditor => Tree {
            nodes: vec![
                (
                    NodeDef {
                        title: "Render Layers",
                        color: HEADER_COL_INPUT,
                        width: 160.0,
                        rows: vec![Output("Image", Sock::Color), Output("Alpha", Sock::Float), Dropdown("Scene"), Dropdown("ViewLayer")],
                    },
                    (-220.0, -40.0),
                ),
                (NodeDef { title: "Composite", color: HEADER_COL_OUTPUT, width: 140.0, rows: vec![Check("Use Alpha", true), Input("Image", Sock::Color, Field::Label, Bind::None, 0.0)] }, (100.0, -80.0)),
                (NodeDef { title: "Viewer", color: HEADER_COL_OUTPUT, width: 140.0, rows: vec![Check("Use Alpha", true), Input("Image", Sock::Color, Field::Label, Bind::None, 0.0)] }, (100.0, 40.0)),
            ],
            links: vec![(0, "Image", 1, "Image"), (0, "Image", 2, "Image")],
        },
        EditorKind::GeometryNodeEditor => Tree {
            nodes: vec![
                (NodeDef { title: "Group Input", color: HEADER_COL_GROUP, width: 140.0, rows: vec![Output("Geometry", Sock::Geometry), Output("", Sock::Virtual)] }, (-220.0, -20.0)),
                (NodeDef { title: "Group Output", color: HEADER_COL_GROUP, width: 140.0, rows: vec![Input("Geometry", Sock::Geometry, Field::Label, Bind::None, 0.0), Input("", Sock::Virtual, Field::Label, Bind::None, 0.0)] }, (80.0, -20.0)),
            ],
            links: vec![(0, "Geometry", 1, "Geometry")],
        },
        _ if world => Tree {
            nodes: vec![
                (
                    NodeDef {
                        title: "Background",
                        color: HEADER_COL_SHADER,
                        width: 140.0,
                        rows: vec![Output("Background", Sock::Shader), Input("Color", Sock::Color, Field::Color, Bind::WorldColor, 0.0), Input("Strength", Sock::Float, Field::Number, Bind::WorldStrength, 0.0)],
                    },
                    (-180.0, -40.0),
                ),
                (NodeDef { title: "World Output", color: HEADER_COL_OUTPUT, width: 140.0, rows: vec![Dropdown("All"), Input("Surface", Sock::Shader, Field::Label, Bind::None, 0.0), Input("Volume", Sock::Shader, Field::Label, Bind::None, 0.0)] }, (40.0, -40.0)),
            ],
            links: vec![(0, "Background", 1, "Surface")],
        },
        _ => Tree {
            nodes: vec![
                (principled(open), (-200.0, -150.0)),
                (
                    NodeDef {
                        title: "Material Output",
                        color: HEADER_COL_OUTPUT,
                        width: 140.0,
                        rows: vec![
                            Dropdown("All"),
                            Input("Surface", Sock::Shader, Field::Label, Bind::None, 0.0),
                            Input("Volume", Sock::Shader, Field::Label, Bind::None, 0.0),
                            Input("Displacement", Sock::Vector, Field::Label, Bind::None, 0.0),
                            Input("Thickness", Sock::Float, Field::Label, Bind::None, 0.0),
                        ],
                    },
                    (100.0, -150.0),
                ),
            ],
            links: vec![(0, "BSDF", 1, "Surface")],
        },
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    None,
    Pan(i32, i32),
    Press { x: i32, y: i32, on_node: bool, extend: bool },
    Move { x: i32, y: i32, orig: Vec<(f32, f32)>, release_confirms: bool },
    Box { x0: i32, y0: i32, x1: i32, y1: i32, extend: bool },
}

pub struct State {
    pub kind: EditorKind,
    id: u64,
    /// Node-space point shown at the body's centre, and pixels per unit.
    pub center: (f32, f32),
    pub zoom: f32,
    /// Node positions (top-left, node space; y grows down), selection, active.
    pub pos: Vec<(f32, f32)>,
    pub selected: Vec<bool>,
    pub active: Option<usize>,
    /// Shader editor: Object or World.
    pub world: bool,
    pub use_nodes: bool,
    panels: [bool; 8],
    /// The colour whose RGB sliders are shown under its swatch.
    color_open: Option<Bind>,
    drag: Drag,
    mouse: (i32, i32),
    want_frame: bool,
    /// Field rectangles drawn last frame, by label ("Roughness"), for tests.
    pub fields: Vec<(String, Rect)>,
    undo_for: Option<u64>,
}

impl State {
    pub fn new(kind: EditorKind) -> State {
        State {
            kind,
            id: anim::editor_id(),
            center: (0.0, 0.0),
            zoom: 1.0,
            pos: Vec::new(),
            selected: Vec::new(),
            active: None,
            world: false,
            use_nodes: true,
            panels: [false; 8],
            color_open: None,
            drag: Drag::None,
            mouse: (0, 0),
            want_frame: true,
            fields: Vec::new(),
            undo_for: None,
        }
    }
}

// ---------------------------------------------------------------- binding

fn material_index(d: &Doc) -> Option<usize> {
    let a = d.active()?;
    let o = &d.scene.objects[a];
    o.materials.first().copied().filter(|&m| m < d.scene.materials.len())
}

fn get_value(d: &Doc, b: Bind) -> Option<f32> {
    let m = material_index(d).map(|i| &d.scene.materials[i]);
    Some(match b {
        Bind::Metallic => m?.metallic,
        Bind::Roughness => m?.roughness,
        Bind::Ior => m?.ior,
        Bind::Alpha => m?.base_color[3],
        Bind::TransmissionWeight => m?.transmission,
        Bind::EmissionStrength => m?.emission_strength,
        Bind::WorldStrength => d.scene.world.strength,
        _ => return None,
    })
}

fn set_value(d: &mut Doc, b: Bind, v: f32) {
    if matches!(b, Bind::WorldStrength) {
        d.scene.world.strength = v.max(0.0);
        return;
    }
    let Some(i) = material_index(d) else { return };
    let m = &mut d.scene.materials[i];
    match b {
        Bind::Metallic => m.metallic = v.clamp(0.0, 1.0),
        Bind::Roughness => m.roughness = v.clamp(0.0, 1.0),
        Bind::Ior => m.ior = v.max(1.0),
        Bind::Alpha => m.base_color[3] = v.clamp(0.0, 1.0),
        Bind::TransmissionWeight => m.transmission = v.clamp(0.0, 1.0),
        Bind::EmissionStrength => m.emission_strength = v.max(0.0),
        _ => {}
    }
}

fn get_color(d: &Doc, b: Bind) -> Option<[f32; 3]> {
    let m = material_index(d).map(|i| &d.scene.materials[i]);
    Some(match b {
        Bind::BaseColor => {
            let c = m?.base_color;
            [c[0], c[1], c[2]]
        }
        Bind::EmissionColor => m?.emission.to_array(),
        Bind::WorldColor => d.scene.world.color.to_array(),
        _ => return None,
    })
}

fn set_color(d: &mut Doc, b: Bind, c: [f32; 3]) {
    if b == Bind::WorldColor {
        d.scene.world.color = sex_3d::Vec3::from_array(c);
        return;
    }
    let Some(i) = material_index(d) else { return };
    let m = &mut d.scene.materials[i];
    match b {
        Bind::BaseColor => {
            m.base_color[0] = c[0];
            m.base_color[1] = c[1];
            m.base_color[2] = c[2];
        }
        Bind::EmissionColor => m.emission = sex_3d::Vec3::from_array(c),
        _ => {}
    }
}

fn display(c: [f32; 3]) -> u32 {
    let b = |v: f32| (srgb_encode(v) * 255.0 + 0.5) as u32;
    b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
}

// ---------------------------------------------------------------- header

fn menus(s: &State) -> (Vec<MenuItem>, Vec<MenuItem>, Vec<MenuItem>, Vec<MenuItem>) {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    let view = vec![m("Frame All", "Home", "node.view_all"), m("Frame Selected", "Numpad .", "node.view_selected"), MenuItem::Separator, MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area"))];
    let select = vec![m("All", "A", "node.select_all"), m("None", "Alt A", "node.select_none"), m("Invert", "Ctrl I", "node.select_invert"), MenuItem::Separator, m("Box Select", "B", "node.select_box")];
    let add_cat = |name: &str| MenuItem::Sub(name.into(), vec![MenuItem::op(&format!("{name} nodes"), "", Op::with("node.add_node", name))]);
    let add = match s.kind {
        EditorKind::CompositorEditor => ["Input", "Output", "Color", "Filter", "Converter", "Matte", "Distort", "Utilities"].iter().map(|c| add_cat(c)).collect(),
        EditorKind::GeometryNodeEditor => ["Attribute", "Input", "Output", "Geometry", "Curve", "Mesh", "Point", "Utilities"].iter().map(|c| add_cat(c)).collect(),
        _ => ["Input", "Output", "Shader", "Displacement", "Color", "Texture", "Vector", "Converter"].iter().map(|c| add_cat(c)).collect(),
    };
    let node = vec![
        m("Move", "G", "node.translate"),
        MenuItem::Separator,
        MenuItem::op("Delete", "X", Op::new("node.delete")),
        MenuItem::op("Duplicate", "Shift D", Op::new("node.duplicate")),
        MenuItem::Separator,
        MenuItem::op("Make Group", "Ctrl G", Op::new("node.group_make")),
    ];
    (view, select, add, node)
}

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let y = hr.y + 3;
    let h = hr.h - 6;
    let mut x = hr.x;
    if s.kind == EditorKind::ShaderEditor {
        let r = Rect::new(x, y, 76, h);
        if ui.dropdown(r, if s.world { "World" } else { "Object" }) {
            ui.open_menu(r, "Shader Type", vec![MenuItem::op("Object", "", anim::mail("node.shader_type", s.id, "OBJECT")), MenuItem::op("World", "", anim::mail("node.shader_type", s.id, "WORLD")), MenuItem::op("Line Style", "", anim::mail("node.shader_type", s.id, "LINESTYLE"))]);
        }
        x = r.right() + 8;
    }
    let (v, se, a, n) = menus(s);
    let view = move |_: &Doc| v.clone();
    let sel = move |_: &Doc| se.clone();
    let add = move |_: &Doc| a.clone();
    let node = move |_: &Doc| n.clone();
    let list: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 4] = [("View", &view), ("Select", &sel), ("Add", &add), ("Node", &node)];
    x = header_menus(ui, hr, x, &list, d) + 10;
    let ur = Rect::new(x, y, 90, h);
    ui.checkbox(ur, "Use Nodes", &mut s.use_nodes);
    x = ur.right() + 10;
    match s.kind {
        EditorKind::ShaderEditor if !s.world => {
            let Some(a) = d.active() else { return };
            let sr = Rect::new(x, y, 70, h);
            if ui.dropdown(sr, "Slot 1") {
                let n = d.scene.objects[a].materials.len().max(1);
                ui.open_menu(sr, "", (1..=n).map(|i| MenuItem::op(&format!("Slot {i}"), "", Op::new("object.material_slot_select"))).collect());
            }
            x = sr.right() + 6;
            match material_index(d) {
                Some(mi) => {
                    let mr = Rect::new(x, y, 160, h);
                    let name = d.scene.materials[mi].name.clone();
                    if ui.dropdown(mr, &name) {
                        let items = d.scene.materials.iter().enumerate().map(|(i, m)| MenuItem::op(&m.name, "", anim::mail("node.material_pick", s.id, &i.to_string()))).collect();
                        ui.open_menu(mr, "Browse Material", items);
                    }
                    let swatch = d.scene.materials[mi].base_color;
                    ui.p.circle((mr.right() - 26) as f32, (mr.y + mr.h / 2) as f32, 5.0, display([swatch[0], swatch[1], swatch[2]]));
                }
                None => {
                    let nr = Rect::new(x, y, 70, h);
                    if ui.button(nr, "+ New") {
                        d.push_undo("New Material");
                        let name = sex_3d::scene::unique_name("Material", |n| d.scene.materials.iter().any(|m| m.name == n));
                        let mi = d.scene.add_material(Material { name, ..Default::default() });
                        if let Some(a) = d.active() {
                            d.scene.objects[a].materials.push(mi);
                        }
                        d.touch();
                    }
                }
            }
        }
        EditorKind::ShaderEditor => {
            ui.dropdown(Rect::new(x, y, 120, h), "World");
        }
        EditorKind::GeometryNodeEditor => {
            ui.dropdown(Rect::new(x, y, 140, h), "Geometry Nodes");
        }
        _ => {
            ui.flat_button(Rect::new(x, y, 70, h), "Backdrop", false);
        }
    }
}

// ---------------------------------------------------------------- layout

/// A laid-out node in screen space.
struct Placed {
    rect: Rect,
    /// (row rect, row) in screen space.
    rows: Vec<(Rect, RowDef)>,
}

fn to_screen(s: &State, r: Rect, p: (f32, f32)) -> (f32, f32) {
    let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    (cx + (p.0 - s.center.0) * s.zoom, cy + (p.1 - s.center.1) * s.zoom)
}

fn node_height(def: &NodeDef) -> f32 {
    HEAD + 6.0 + def.rows.len() as f32 * ROW + 6.0
}

fn place(s: &State, r: Rect, t: &Tree) -> Vec<Placed> {
    t.nodes
        .iter()
        .enumerate()
        .map(|(i, (def, _))| {
            let (x, y) = to_screen(s, r, s.pos[i]);
            let w = def.width * s.zoom;
            let h = node_height(def) * s.zoom;
            let rect = Rect::new(x.round() as i32, y.round() as i32, w.round() as i32, h.round() as i32);
            let mut ry = y + (HEAD + 6.0) * s.zoom;
            let rows = def
                .rows
                .iter()
                .map(|row| {
                    let rr = Rect::new(rect.x, ry.round() as i32, rect.w, (ROW * s.zoom).round() as i32);
                    ry += ROW * s.zoom;
                    (rr, row.clone())
                })
                .collect();
            Placed { rect, rows }
        })
        .collect()
}

/// The interactive part of a row (field, dropdown, panel toggle), if any.
fn row_widget(s: &State, rr: Rect, row: &RowDef) -> Option<Rect> {
    let pad = (8.0 * s.zoom).round() as i32;
    let fh = ((ROW - 3.0) * s.zoom).round() as i32;
    let inner = Rect::new(rr.x + pad, rr.y + 1, rr.w - 2 * pad, fh);
    match row {
        RowDef::Input(_, _, f, _, _) if *f != Field::Label => Some(inner),
        RowDef::Dropdown(_) | RowDef::Panel(..) | RowDef::Check(..) => Some(inner),
        _ => None,
    }
}

fn socket_pos(placed: &[Placed], node: usize, name: &str, output: bool) -> Option<(f32, f32)> {
    let p = placed.get(node)?;
    for (rr, row) in &p.rows {
        let hit = match row {
            RowDef::Output(n, _) => output && *n == name,
            RowDef::Input(n, ..) => !output && *n == name,
            _ => false,
        };
        if hit {
            let x = if output { p.rect.right() } else { p.rect.x };
            return Some((x as f32, rr.y as f32 + rr.h as f32 / 2.0));
        }
    }
    None
}

// ---------------------------------------------------------------- body

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    for (op, arg) in anim::take_mail(d, s.id) {
        match op.as_str() {
            "node.view_all" | "node.view_selected" => s.want_frame = true,
            "node.select_all" => s.selected.iter_mut().for_each(|v| *v = true),
            "node.select_none" => s.selected.iter_mut().for_each(|v| *v = false),
            "node.select_invert" => s.selected.iter_mut().for_each(|v| *v = !*v),
            "node.select_box" => {}
            "node.translate" => start_move(s, false),
            "node.shader_type" => {
                if arg == "LINESTYLE" {
                    anim::report(d, ReportKind::Warning, "Line Style shaders are not available yet");
                } else {
                    s.world = arg == "WORLD";
                    s.pos.clear();
                    s.want_frame = true;
                }
            }
            "node.material_pick" => {
                if let (Ok(mi), Some(a)) = (arg.parse::<usize>(), d.active()) {
                    d.push_undo("Assign Material");
                    let o = &mut d.scene.objects[a];
                    if o.materials.is_empty() {
                        o.materials.push(mi);
                    } else {
                        o.materials[0] = mi;
                    }
                    d.touch();
                }
            }
            _ => {}
        }
    }
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    let p = &mut *ui.p;
    p.fill(r, t::NODE_BG);
    draw_grid(s, p, r);
    s.fields.clear();
    let no_material = s.kind == EditorKind::ShaderEditor && !s.world && material_index(d).is_none();
    if !s.use_nodes || no_material {
        let msg = if no_material { "The active object has no material: + New in the header adds one" } else { "Use Nodes is off" };
        p.label(r, msg, t::TEXT_DIM, Align::Center);
        return;
    }
    let tr = tree(s.kind, s.world, &s.panels, s.color_open);
    if s.pos.len() != tr.nodes.len() {
        s.pos = tr.nodes.iter().map(|n| n.1).collect();
        s.selected = vec![false; tr.nodes.len()];
        s.active = None;
    }
    if s.want_frame && r.w > 0 {
        s.want_frame = false;
        frame_all(s, &tr, r);
    }
    let placed = place(s, r, &tr);

    // ---- canvas input (anything not on a field)
    canvas_input(s, d, ui, r, &placed);
    let placed = place(s, r, &tr);

    // ---- links under the nodes
    let p = &mut *ui.p;
    for &(a, out, b, inp) in &tr.links {
        if let (Some(p0), Some(p1)) = (socket_pos(&placed, a, out, true), socket_pos(&placed, b, inp, false)) {
            link(p, p0, p1, s.zoom);
        }
    }
    // ---- nodes
    let text = s.zoom >= 0.55;
    for (i, pl) in placed.iter().enumerate() {
        let def = &tr.nodes[i].0;
        let rad = (5.0 * s.zoom).round().max(2.0) as i32;
        let n = pl.rect;
        ui.p.round_rect_alpha(Rect::new(n.x + 2, n.y + 3, n.w, n.h), rad, 0x000000, 80);
        let outline = if s.active == Some(i) && s.selected[i] { Some(0xffffff) } else if s.selected[i] { Some(0xed5700) } else { None };
        if let Some(c) = outline {
            ui.p.round_rect(Rect::new(n.x - 2, n.y - 2, n.w + 4, n.h + 4), rad + 2, c);
        }
        ui.p.round_rect(n, rad, NODE_BODY);
        let hh = (HEAD * s.zoom).round() as i32;
        ui.p.round_rect(Rect::new(n.x, n.y, n.w, hh + rad), rad, def.color);
        ui.p.fill(Rect::new(n.x, n.y + hh, n.w, rad + 1), NODE_BODY);
        if text {
            ui.p.arrow(n.x as f32 + 11.0 * s.zoom, n.y as f32 + hh as f32 / 2.0, 7.0, true, 0xdddddd);
            ui.p.label(Rect::new(n.x + (20.0 * s.zoom) as i32, n.y, n.w - (24.0 * s.zoom) as i32, hh), def.title, 0xeeeeee, Align::Left);
        }
        ui.p.push_clip(Rect::new(n.x - 8, n.y, n.w + 16, n.h));
        for (rr, row) in &pl.rows {
            draw_row(s, d, ui, *rr, row, text);
        }
        ui.p.pop_clip();
    }
    if let Drag::Box { x0, y0, x1, y1, .. } = s.drag {
        let b = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
        ui.p.fill_alpha(b, 0xffffff, 18);
        ui.p.frame(b, 0xbbbbbb);
    }
    if ui.st.active.is_none() {
        s.undo_for = None;
    }
}

fn draw_grid(s: &State, p: &mut Painter, r: Rect) {
    // Blender 4's dotted grid; dots every 20 units (coarser when zoomed out).
    let mut step = 20.0 * s.zoom;
    while step < 12.0 {
        step *= 2.0;
    }
    let (ox, oy) = to_screen(s, r, (0.0, 0.0));
    let x0 = r.x as f32 + (ox - r.x as f32).rem_euclid(step);
    let y0 = r.y as f32 + (oy - r.y as f32).rem_euclid(step);
    let mut y = y0;
    while y < r.bottom() as f32 {
        let mut x = x0;
        while x < r.right() as f32 {
            p.fill(Rect::new(x as i32, y as i32, 2, 2), 0x353535);
            x += step;
        }
        y += step;
    }
}

fn link(p: &mut Painter, a: (f32, f32), b: (f32, f32), zoom: f32) {
    let dx = ((b.0 - a.0).abs() * 0.5).max(40.0 * zoom);
    let (c1, c2) = ((a.0 + dx, a.1), (b.0 - dx, b.1));
    let n = 32;
    let mut prev = a;
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let x = u * u * u * a.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * b.0;
        let y = u * u * u * a.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * b.1;
        p.line(prev.0, prev.1 + 1.0, x, y + 1.0, 0x101010);
        p.line(prev.0, prev.1 - 0.5, x, y - 0.5, 0xb4b4b4);
        p.line(prev.0, prev.1 + 0.5, x, y + 0.5, 0xb4b4b4);
        prev = (x, y);
    }
}

fn socket(p: &mut Painter, x: f32, y: f32, ty: Sock, zoom: f32) {
    let r = (4.5 * zoom).max(2.5);
    if ty == Sock::Virtual {
        p.ring(x, y, r, 0x9a9a9a);
        return;
    }
    p.circle(x, y, r + 1.0, 0x000000);
    p.circle(x, y, r, sock_color(ty));
}

fn draw_row(s: &mut State, d: &mut Doc, ui: &mut Ui, rr: Rect, row: &RowDef, text: bool) {
    let cy = rr.y as f32 + rr.h as f32 / 2.0;
    let pad = (10.0 * s.zoom).round() as i32;
    let w = row_widget(s, rr, row);
    match row {
        RowDef::Output(name, ty) => {
            socket(ui.p, rr.right() as f32, cy, *ty, s.zoom);
            if text {
                ui.p.label(Rect::new(rr.x + pad, rr.y, rr.w - 2 * pad, rr.h), name, t::TEXT, Align::Right);
            }
        }
        RowDef::Dropdown(label) => {
            if let (Some(w), true) = (w, text) {
                if ui.dropdown(w, label) {
                    anim::report(d, ReportKind::Warning, &format!("{label}: other choices are not available yet"));
                }
            }
        }
        RowDef::Check(label, on) => {
            if let (Some(w), true) = (w, text) {
                let mut v = *on;
                ui.checkbox(w, label, &mut v);
            }
        }
        RowDef::Panel(label, i) => {
            let Some(w) = w else { return };
            ui.p.fill(Rect::new(rr.x + 1, rr.y, rr.w - 2, 1), 0x262626);
            let open = s.panels[*i];
            if text {
                ui.p.arrow(w.x as f32 + 5.0, cy, 7.0, open, t::TEXT_DIM);
                ui.p.label(Rect::new(w.x + 14, rr.y, w.w - 14, rr.h), label, t::TEXT, Align::Left);
            }
            if ui.clicked(w) {
                s.panels[*i] = !open;
            }
        }
        RowDef::Input(name, ty, field, bind, fixed) => {
            if !matches!(field, Field::Channel(_)) {
                socket(ui.p, rr.x as f32, cy, *ty, s.zoom);
            }
            let Some(w) = w.filter(|_| text) else {
                if text {
                    ui.p.label(Rect::new(rr.x + pad, rr.y, rr.w - 2 * pad, rr.h), name, t::TEXT, Align::Left);
                }
                return;
            };
            s.fields.push((name.to_string(), w));
            let wid = id(&format!("node{}/{}/{name}", s.id, *bind as u32));
            match field {
                Field::Color => {
                    let c = get_color(d, *bind).unwrap_or([1.0; 3]);
                    let half = w.w / 2;
                    ui.p.label(Rect::new(w.x + 6, w.y, half - 6, w.h), name, t::TEXT, Align::Left);
                    let sw = Rect::new(w.x + half, w.y, w.w - half, w.h);
                    ui.p.round_rect(sw, 4, display(c));
                    if ui.clicked(sw) {
                        s.color_open = if s.color_open == Some(*bind) { None } else { Some(*bind) };
                    }
                }
                Field::Channel(k) => {
                    // Inline R/G/B sliders stand in for Blender's colour picker popup.
                    let mut c = get_color(d, *bind).unwrap_or([1.0; 3]);
                    if slider(ui, wid, w, name, &mut c[*k as usize], 0.0, 1.0, true, true) {
                        undo_once(s, d, wid);
                        set_color(d, *bind, c);
                        d.touch();
                    }
                }
                Field::Factor | Field::Number => {
                    let bound = get_value(d, *bind);
                    let mut v = bound.unwrap_or(*fixed);
                    let (min, max) = if *field == Field::Factor { (0.0, 1.0) } else { (f32::MIN, f32::MAX) };
                    if slider(ui, wid, w, name, &mut v, min, max, *field == Field::Factor, bound.is_some()) {
                        if bound.is_some() {
                            undo_once(s, d, wid);
                            set_value(d, *bind, v);
                            d.touch();
                        } else {
                            anim::report(d, ReportKind::Warning, &format!("{name} is not available yet"));
                        }
                    }
                }
                Field::Label => {}
            }
        }
    }
}

/// One undo step per drag or typed value.
fn undo_once(s: &mut State, d: &mut Doc, wid: u64) {
    if s.undo_for != Some(wid) {
        d.push_undo("Change Material");
        s.undo_for = Some(wid);
    }
}

/// Blender's slider: drag to change, click to type a value. Returns true when
/// the value changed.
#[allow(clippy::too_many_arguments)]
fn slider(ui: &mut Ui, wid: u64, r: Rect, label: &str, value: &mut f32, min: f32, max: f32, fill: bool, enabled: bool) -> bool {
    let hover = ui.hovered(r);
    let editing = matches!(&ui.st.editing, Some((e, _)) if *e == wid);
    let dragging = ui.st.active == Some(wid);
    let text_col = if enabled { t::TEXT } else { t::TEXT_DIM };
    if editing {
        ui.p.round_rect(r, 4, t::BUTTON_DARK);
        let text = ui.st.editing.as_ref().map(|e| e.1.clone()).unwrap_or_default();
        ui.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), &format!("{text}|"), t::TEXT_HI, Align::Left);
        let mut commit = None;
        match ui.input {
            Some(Input::Key { key, text: ch, .. }) => {
                ui.input = None;
                match key {
                    Key::Enter | Key::NumEnter => commit = Some(text),
                    Key::Esc => ui.st.editing = None,
                    Key::Backspace => {
                        if let Some(e) = &mut ui.st.editing {
                            e.1.pop();
                        }
                    }
                    _ => {
                        let c = ch.or(match key {
                            Key::Char(c) => Some(c),
                            Key::NumDot => Some('.'),
                            Key::NumMinus => Some('-'),
                            Key::Num(n) => char::from_digit(n as u32, 10),
                            _ => None,
                        });
                        if let (Some(c), Some(e)) = (c, &mut ui.st.editing) {
                            if c.is_ascii_graphic() {
                                e.1.push(c);
                            }
                        }
                    }
                }
            }
            Some(Input::Down { x, y, .. }) if !r.contains(x, y) => commit = Some(text),
            _ => {}
        }
        if let Some(text) = commit {
            ui.st.editing = None;
            if let Some(v) = eval_number(&text) {
                *value = v.clamp(min, max);
                return true;
            }
        }
        return false;
    }
    ui.p.round_rect(r, 4, if dragging { t::BUTTON_HOVER } else if hover { t::BUTTON_HOVER } else { t::BUTTON });
    if fill {
        let f = ((*value - min) / (max - min)).clamp(0.0, 1.0);
        let fw = (r.w as f32 * f).round() as i32;
        if fw > 0 {
            ui.p.push_clip(Rect::new(r.x, r.y, fw, r.h));
            ui.p.round_rect(r, 4, t::SELECTED);
            ui.p.pop_clip();
        }
    }
    let shown = format!("{:.3}", value);
    ui.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), label, text_col, Align::Left);
    ui.p.label(Rect::new(r.x + 6, r.y, r.w - 12, r.h), &shown, text_col, Align::Right);
    let mut changed = false;
    match ui.input {
        Some(Input::Down { x, y, button: Button::Left, .. }) if r.contains(x, y) && ui.p.clip().contains(x, y) => {
            ui.input = None;
            ui.st.active = Some(wid);
            ui.st.drag_origin = (x, y);
            ui.st.drag_start_value = *value;
            ui.st.drag_moved = false;
        }
        Some(Input::Move { x, .. }) if dragging => {
            ui.input = None;
            let dx = x - ui.st.drag_origin.0;
            if dx.abs() > 2 {
                ui.st.drag_moved = true;
            }
            if ui.st.drag_moved {
                let per_px = if fill { (max - min) / r.w.max(1) as f32 } else { 0.01 };
                *value = (ui.st.drag_start_value + dx as f32 * per_px).clamp(min, max);
                changed = true;
            }
        }
        Some(Input::Up { button: Button::Left, .. }) if dragging => {
            ui.input = None;
            ui.st.active = None;
            if !ui.st.drag_moved {
                ui.st.editing = Some((wid, trim_number(*value)));
            }
        }
        _ => {}
    }
    changed
}

// ---------------------------------------------------------------- canvas

fn frame_all(s: &mut State, t: &Tree, r: Rect) {
    let mut lo = (f32::INFINITY, f32::INFINITY);
    let mut hi = (f32::NEG_INFINITY, f32::NEG_INFINITY);
    for (i, (def, _)) in t.nodes.iter().enumerate() {
        let p = s.pos[i];
        lo = (lo.0.min(p.0), lo.1.min(p.1));
        hi = (hi.0.max(p.0 + def.width), hi.1.max(p.1 + node_height(def)));
    }
    if !lo.0.is_finite() {
        return;
    }
    s.center = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
    let zx = r.w as f32 * 0.85 / (hi.0 - lo.0).max(1.0);
    let zy = r.h as f32 * 0.85 / (hi.1 - lo.1).max(1.0);
    s.zoom = zx.min(zy).clamp(0.3, 1.0);
}

fn start_move(s: &mut State, release_confirms: bool) {
    if s.selected.iter().any(|&v| v) {
        s.drag = Drag::Move { x: s.mouse.0, y: s.mouse.1, orig: s.pos.clone(), release_confirms };
    }
}

fn canvas_input(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect, placed: &[Placed]) {
    let Some(input) = ui.input else { return };
    let clip = ui.p.clip();
    let inside = |x: i32, y: i32| r.contains(x, y) && clip.contains(x, y);
    let on_widget = |x: i32, y: i32| placed.iter().any(|p| p.rows.iter().any(|(rr, row)| row_widget(s, *rr, row).is_some_and(|w| w.contains(x, y))));
    let node_at = |x: i32, y: i32| placed.iter().rposition(|p| p.rect.contains(x, y));
    let _ = d;
    match (s.drag.clone(), input) {
        (Drag::Move { x, y, orig, release_confirms }, inp) => {
            match inp {
                Input::Move { x: mx, y: my } => {
                    let (dx, dy) = ((mx - x) as f32 / s.zoom, (my - y) as f32 / s.zoom);
                    for i in 0..s.pos.len() {
                        if s.selected[i] {
                            s.pos[i] = (orig[i].0 + dx, orig[i].1 + dy);
                        }
                    }
                }
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter, .. } => s.drag = Drag::None,
                Input::Up { button: Button::Left, .. } if release_confirms => s.drag = Drag::None,
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    s.pos = orig;
                    s.drag = Drag::None;
                }
                _ => return,
            }
            ui.input = None;
        }
        (Drag::Pan(lx, ly), Input::Move { x, y }) => {
            s.center.0 -= (x - lx) as f32 / s.zoom;
            s.center.1 -= (y - ly) as f32 / s.zoom;
            s.drag = Drag::Pan(x, y);
            ui.input = None;
        }
        (Drag::Pan(..), Input::Up { button: Button::Middle, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Press { x: px, y: py, on_node, extend }, Input::Move { x, y }) => {
            if (x - px).abs() + (y - py).abs() > 3 {
                if on_node {
                    s.mouse = (px, py);
                    start_move(s, true);
                    if let Drag::Move { x: ox, y: oy, orig, .. } = s.drag.clone() {
                        let (dx, dy) = ((x - ox) as f32 / s.zoom, (y - oy) as f32 / s.zoom);
                        for i in 0..s.pos.len() {
                            if s.selected[i] {
                                s.pos[i] = (orig[i].0 + dx, orig[i].1 + dy);
                            }
                        }
                    }
                } else {
                    s.drag = Drag::Box { x0: px, y0: py, x1: x, y1: y, extend };
                }
            }
            ui.input = None;
        }
        (Drag::Press { on_node, extend, .. }, Input::Up { button: Button::Left, .. }) => {
            if !on_node && !extend {
                s.selected.iter_mut().for_each(|v| *v = false);
                s.active = None;
            }
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Box { x0, y0, extend, .. }, Input::Move { x, y }) => {
            s.drag = Drag::Box { x0, y0, x1: x, y1: y, extend };
            ui.input = None;
        }
        (Drag::Box { x0, y0, x1, y1, extend }, Input::Up { button: Button::Left, .. }) => {
            let b = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs() + 1, (y1 - y0).abs() + 1);
            for (i, p) in placed.iter().enumerate() {
                let hit = !p.rect.intersect(&b).is_empty();
                if hit {
                    s.selected[i] = true;
                } else if !extend {
                    s.selected[i] = false;
                }
            }
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::None, Input::Down { x, y, button: Button::Left, mods }) if inside(x, y) && !on_widget(x, y) => {
            match node_at(x, y) {
                Some(i) => {
                    if mods.shift {
                        if s.selected[i] && s.active == Some(i) {
                            s.selected[i] = false;
                        } else {
                            s.selected[i] = true;
                            s.active = Some(i);
                        }
                    } else {
                        if !s.selected[i] {
                            s.selected.iter_mut().for_each(|v| *v = false);
                        }
                        s.selected[i] = true;
                        s.active = Some(i);
                    }
                    s.drag = Drag::Press { x, y, on_node: true, extend: mods.shift };
                }
                None => s.drag = Drag::Press { x, y, on_node: false, extend: mods.shift },
            }
            ui.input = None;
        }
        (Drag::None, Input::Down { x, y, button: Button::Middle, .. }) if inside(x, y) => {
            s.drag = Drag::Pan(x, y);
            ui.input = None;
        }
        (Drag::None, Input::Wheel { x, y, steps, mods }) if inside(x, y) => {
            if mods.ctrl {
                s.center.0 -= steps as f32 * 40.0 / s.zoom;
            } else if mods.shift {
                s.center.1 -= steps as f32 * 40.0 / s.zoom;
            } else {
                let before = (s.center.0 + (x - r.x - r.w / 2) as f32 / s.zoom, s.center.1 + (y - r.y - r.h / 2) as f32 / s.zoom);
                s.zoom = (s.zoom * 1.15f32.powi(steps)).clamp(0.2, 3.0);
                s.center = (before.0 - (x - r.x - r.w / 2) as f32 / s.zoom, before.1 - (y - r.y - r.h / 2) as f32 / s.zoom);
            }
            ui.input = None;
        }
        _ => {}
    }
}

pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    match (key, mods) {
        (Key::Home, Mods::NONE) | (Key::NumDot, Mods::NONE) => s.want_frame = true,
        (Key::Char('a'), Mods::NONE) => s.selected.iter_mut().for_each(|v| *v = true),
        (Key::Char('a'), Mods::ALT) => s.selected.iter_mut().for_each(|v| *v = false),
        (Key::Char('i'), Mods::CTRL) => s.selected.iter_mut().for_each(|v| *v = !*v),
        (Key::Char('g'), Mods::NONE) => start_move(s, false),
        (Key::Char('x'), Mods::NONE) | (Key::Delete, Mods::NONE) => d.queue.push(Op::new("node.delete")),
        (Key::Char('d'), Mods::SHIFT) => d.queue.push(Op::new("node.duplicate")),
        (Key::Char('a'), Mods::SHIFT) => d.queue.push(Op::with("node.add_node", "")),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editors::Editor;
    use crate::App;

    #[test]
    fn roughness_field_edits_the_material() {
        let mut app = App::new(1600, 900);
        app.doc.want_workspace = Some("Shading".into());
        app.frame(Input::Tick);
        app.frame(Input::Tick);
        let area = app.workspaces[app.active_ws].areas().into_iter().find(|a| a.editor.kind() == EditorKind::ShaderEditor).map(|a| a.id).unwrap();
        let fields = match &app.workspaces[app.active_ws].area_mut(area).unwrap().editor {
            Editor::Node(s) => s.fields.clone(),
            _ => unreachable!(),
        };
        let r = fields.iter().find(|f| f.0 == "Roughness").expect("roughness field").1;
        let (x, y) = r.center();
        app.frame(Input::Move { x, y });
        app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Up { x, y, button: Button::Left });
        app.frame(Input::key_mods(Key::Backspace, Mods::NONE));
        app.frame(Input::key_mods(Key::Backspace, Mods::NONE));
        app.frame(Input::key_mods(Key::Backspace, Mods::NONE));
        for c in "0.2".chars() {
            app.frame(Input::Key { key: Key::Char(c), mods: Mods::NONE, text: Some(c) });
        }
        app.frame(Input::key(Key::Enter));
        let mi = app.doc.scene.objects[app.doc.active().unwrap()].materials[0];
        assert!((app.doc.scene.materials[mi].roughness - 0.2).abs() < 1e-6, "{}", app.doc.scene.materials[mi].roughness);
        // Dragging the field changes it too, and Ctrl+Z restores.
        app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
        app.frame(Input::Move { x: x + 40, y });
        app.frame(Input::Up { x: x + 40, y, button: Button::Left });
        assert!(app.doc.scene.materials[mi].roughness > 0.25);
        app.frame(Input::Move { x: 300, y: 300 });
        app.frame(Input::key_mods(Key::Char('z'), Mods::CTRL));
        assert!((app.doc.scene.materials[mi].roughness - 0.2).abs() < 1e-6);
    }

    #[test]
    fn nodes_select_and_move() {
        let mut d = Doc::default();
        let mut s = State::new(EditorKind::CompositorEditor);
        let mut p = Painter::new(800, 600);
        let mut st = crate::ui::UiState::default();
        let r = Rect::new(0, 0, 800, 600);
        let mut frame = |s: &mut State, d: &mut Doc, inp: Option<Input>| {
            if let Some(pos) = inp.and_then(|i| i.pos()) {
                st.mouse = pos;
            }
            let mut ui = Ui { p: &mut p, st: &mut st, input: inp };
            body(s, d, &mut ui, r);
        };
        frame(&mut s, &mut d, None);
        let tr = tree(s.kind, false, &s.panels, None);
        let placed = place(&s, r, &tr);
        let n = placed[0].rect;
        let (x, y) = (n.x + 20, n.y + 8);
        frame(&mut s, &mut d, Some(Input::Down { x, y, button: Button::Left, mods: Mods::NONE }));
        assert!(s.selected[0] && s.active == Some(0));
        let before = s.pos[0];
        frame(&mut s, &mut d, Some(Input::Move { x: x + 50, y }));
        frame(&mut s, &mut d, Some(Input::Up { x: x + 50, y, button: Button::Left }));
        assert!((s.pos[0].0 - before.0 - 50.0 / s.zoom).abs() < 0.01);
        // Box select from empty space catches the others.
        frame(&mut s, &mut d, Some(Input::Down { x: 790, y: 590, button: Button::Left, mods: Mods::NONE }));
        frame(&mut s, &mut d, Some(Input::Move { x: 5, y: 5 }));
        frame(&mut s, &mut d, Some(Input::Up { x: 5, y: 5, button: Button::Left }));
        assert!(s.selected.iter().all(|&v| v));
    }
}
