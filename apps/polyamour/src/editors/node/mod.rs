// SPDX-License-Identifier: GPL-3.0-only
//! Blender's node editors: the Shader Editor (the active object's material
//! or the World), the Compositor and the Geometry Node Editor, on one
//! editable canvas drawn from the node trees' data (`sex_3d::nodes`).
//!
//! Nodes are laid out like Blender's (header, outputs, buttons, inputs,
//! panels), with socket colours by type and value fields by subtype. The
//! keymap is Blender's: Shift+A adds (with search), drag from a socket links
//! (drop on an input replaces its link, drag from a linked input picks the
//! link up), Ctrl+right-drag cuts links, Shift+right-drag adds reroutes,
//! X deletes, Ctrl+X dissolves, Shift+D duplicates, Ctrl+C/V copy and paste,
//! G moves, M mutes, H collapses, Ctrl+H hides unused sockets, F makes links,
//! Ctrl+J frames, Alt+P unframes, Ctrl+G groups, Tab enters/leaves a group,
//! Ctrl+Alt+G ungroups, Home frames all, Numpad . frames the selection,
//! N toggles the sidebar, L / Shift+L select linked, Ctrl+F finds a node.
//! Middle-drag pans, the wheel zooms at the pointer. Ctrl+Alt+left-drag
//! (Ctrl+middle once Alt+left is middle-button emulation) also cuts, and
//! Ctrl+Shift+Alt+left adds reroutes, for mice without a right button drag.
//!
//! Every change is one undo step and refreshes the viewports and F12.
use super::header_menus;
use super::properties::picker::{self, Picker};
use super::properties::ColorTarget;
use crate::anim::{self, ReportKind};
use crate::doc::Doc;
use crate::draw::{Align, Painter, Rect};
use crate::editors::EditorKind;
use crate::input::{Button, Input, Key, Mods};
use crate::nodetree::{self, NodeColor, TreeRef};
use crate::theme as t;
use crate::ui::{id, MenuItem, NumOpts, Op, Ui};
use sex_3d::nodes::{edit, enums, update, Link, NodeTree, Prop, SocketType, Subtype, TreeKind, Value};
use sex_3d::scene::Material;
use std::collections::{HashMap, HashSet};

pub mod addmenu;
pub mod layout;
mod sidebar;
#[cfg(test)]
mod tests;
mod widgets;

use layout::{NodeLayout, RowKind, View, HEAD, ROW};

const NODE_BODY: u32 = 0x303030;
const SIDEBAR_W: i32 = 250;

#[derive(Clone, Debug, PartialEq)]
enum Drag {
    None,
    Pan(i32, i32),
    /// Left press, not yet a drag: on a node (and whether it was selected).
    Press { x: i32, y: i32, node: Option<usize>, extend: bool, was_selected: bool },
    Move { x: i32, y: i32, orig: Vec<(usize, [f32; 2])>, release_confirms: bool, key: u64 },
    Box { x0: i32, y0: i32, x1: i32, y1: i32, extend: bool },
    /// A link being dragged: one end fixed on a socket, the other at (x, y).
    Link { from: Option<(usize, usize)>, to: Option<(usize, usize)>, x: i32, y: i32, key: u64 },
    Knife(Vec<(i32, i32)>),
    Reroute(Vec<(i32, i32)>),
    Resize { node: usize, left: bool, x: i32, orig_w: f32, orig_x: f32, key: u64 },
}

pub struct State {
    pub kind: EditorKind,
    pub id: u64,
    /// Node-space point at the canvas centre, and pixels per unit.
    pub center: (f32, f32),
    pub zoom: f32,
    /// Shader editor: World instead of Object.
    pub world: bool,
    /// Material slot shown (Slot dropdown).
    pub slot: usize,
    /// Geometry Node Editor: the node group shown.
    pub geo_group: Option<usize>,
    /// Groups entered with Tab, innermost last.
    pub path: Vec<usize>,
    /// Open node panels: (node name, panel).
    pub open_panels: HashSet<(String, String)>,
    pub sidebar: bool,
    pub side_tab: usize,
    side_open: HashMap<String, bool>,
    side_undo: Option<Option<u64>>,
    side_scroll: i32,
    pub iface_sel: Option<(bool, usize)>,
    pub add_menu: Option<addmenu::AddMenu>,
    drag: Drag,
    pub mouse: (i32, i32),
    /// Frame on the next draw: Some(false) all, Some(true) the selection.
    want_frame: Option<bool>,
    /// Value fields drawn last frame ("Node/Socket" or "Node/prop"), for tests.
    pub fields: Vec<(String, Rect)>,
    /// Last frame's layout and view.
    pub layout: Vec<NodeLayout>,
    pub view: View,
    undo_for: Option<u64>,
    picker: Option<Picker>,
    picker_undo: Option<Option<u64>>,
    pub clipboard: Option<edit::Clipboard>,
    box_armed: bool,
    /// Image path being typed for a node's Open: (node, text).
    open_path: Option<(usize, String)>,
    ramp_active: HashMap<String, usize>,
    curve_chan: HashMap<String, usize>,
    curve_sel: HashMap<String, usize>,
    drag_key: u64,
}

impl State {
    pub fn new(kind: EditorKind) -> State {
        State {
            kind,
            id: anim::editor_id(),
            center: (0.0, 0.0),
            zoom: 1.0,
            world: false,
            slot: 0,
            geo_group: None,
            path: Vec::new(),
            open_panels: HashSet::new(),
            sidebar: false,
            side_tab: 0,
            side_open: HashMap::new(),
            side_undo: None,
            side_scroll: 0,
            iface_sel: None,
            add_menu: None,
            drag: Drag::None,
            mouse: (0, 0),
            want_frame: Some(false),
            fields: Vec::new(),
            layout: Vec::new(),
            view: View { center: (0.0, 0.0), zoom: 1.0, region: Rect::default() },
            undo_for: None,
            picker: None,
            picker_undo: None,
            clipboard: None,
            box_armed: false,
            open_path: None,
            ramp_active: HashMap::new(),
            curve_chan: HashMap::new(),
            curve_sel: HashMap::new(),
            drag_key: 1,
        }
    }
    fn next_key(&mut self) -> u64 {
        self.drag_key += 1;
        id(&format!("node{}/drag/{}", self.id, self.drag_key))
    }
    /// Whether a modal action (grab, link drag, knife, add menu) is running.
    pub fn is_modal(&self) -> bool {
        !matches!(self.drag, Drag::None) || self.add_menu.is_some()
    }
}

pub fn is_modal(s: &State) -> bool {
    s.is_modal()
}

// ---------------------------------------------------------------- context

fn tree_kind(k: EditorKind) -> TreeKind {
    match k {
        EditorKind::CompositorEditor => TreeKind::Compositor,
        EditorKind::GeometryNodeEditor => TreeKind::Geometry,
        _ => TreeKind::Shader,
    }
}

/// The active object's material in the shown slot.
pub fn material_index(s: &State, d: &Doc) -> Option<usize> {
    let a = d.active()?;
    let o = &d.scene.objects[a];
    o.materials.get(s.slot.min(o.materials.len().saturating_sub(1))).copied().filter(|&m| m < d.scene.materials.len())
}

/// The tree this editor shows (it may not exist yet: no Use Nodes).
pub fn tree_ref(s: &State, d: &Doc) -> Option<TreeRef> {
    if let Some(&g) = s.path.last() {
        if g < d.scene.node_groups.len() {
            return Some(TreeRef::Group(g));
        }
    }
    match s.kind {
        EditorKind::CompositorEditor => Some(TreeRef::Compositor),
        EditorKind::GeometryNodeEditor => s.geo_group.filter(|&g| g < d.scene.node_groups.len()).map(TreeRef::Group),
        _ if s.world => Some(TreeRef::World),
        _ => material_index(s, d).map(TreeRef::Material),
    }
}

fn display(c: [f32; 3]) -> u32 {
    let b = |v: f32| (sex_3d::render::srgb_encode(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u32;
    b(c[0]) << 16 | b(c[1]) << 8 | b(c[2])
}

// ---------------------------------------------------------------- header

fn menus(s: &State, d: &Doc) -> (Vec<MenuItem>, Vec<MenuItem>, Vec<MenuItem>, Vec<MenuItem>) {
    let m = |l: &str, k: &str, op: &str| MenuItem::op(l, k, anim::mail(op, s.id, ""));
    let ma = |l: &str, k: &str, op: &str, a: &str| MenuItem::op(l, k, anim::mail(op, s.id, a));
    let view = vec![
        MenuItem::Check("Sidebar".into(), s.sidebar, anim::mail("node.sidebar", s.id, "")),
        MenuItem::Separator,
        m("Frame All", "Home", "node.view_all"),
        m("Frame Selected", "Numpad .", "node.view_selected"),
        MenuItem::Separator,
        MenuItem::op("Toggle Maximize Area", "Ctrl Space", Op::new("screen.screen_full_area")),
    ];
    let select = vec![
        m("All", "A", "node.select_all"),
        m("None", "Alt A", "node.select_none"),
        m("Invert", "Ctrl I", "node.select_invert"),
        MenuItem::Separator,
        m("Box Select", "B", "node.select_box"),
        MenuItem::Separator,
        m("Linked From", "Shift L", "node.select_linked_from"),
        m("Linked To", "L", "node.select_linked_to"),
        MenuItem::Separator,
        m("Find Node...", "Ctrl F", "node.find_node"),
    ];
    let current = tree_ref(s, d).and_then(|r| r.group());
    let cats = addmenu::categories(tree_kind(s.kind), &d.scene.node_groups, current);
    let mut add = vec![ma("Search...", "", "node.add_search", ""), MenuItem::Separator];
    for (name, entries) in cats {
        let items: Vec<MenuItem> = if entries.is_empty() { vec![MenuItem::Heading("No node groups".into())] } else { entries.iter().map(|e| ma(&e.label, "", "node.add_node", &e.arg())).collect() };
        add.push(MenuItem::Sub(name, items));
    }
    let node = vec![
        m("Move", "G", "node.translate"),
        MenuItem::Separator,
        m("Duplicate", "Shift D", "node.duplicate_move"),
        m("Copy", "Ctrl C", "node.clipboard_copy"),
        m("Paste", "Ctrl V", "node.clipboard_paste"),
        MenuItem::Separator,
        m("Delete", "X", "node.delete"),
        m("Delete with Reconnect", "Ctrl X", "node.delete_reconnect"),
        MenuItem::Separator,
        m("Join in New Frame", "Ctrl J", "node.join"),
        m("Remove from Frame", "Alt P", "node.detach"),
        MenuItem::Separator,
        m("Make Links", "F", "node.link_make"),
        ma("Make and Replace Links", "Shift F", "node.link_make", "REPLACE"),
        m("Cut Links", "Ctrl RMB", "node.links_cut"),
        MenuItem::Separator,
        m("Make Group", "Ctrl G", "node.group_make"),
        m("Ungroup", "Ctrl Alt G", "node.group_ungroup"),
        m("Edit Group", "Tab", "node.group_edit"),
        MenuItem::Separator,
        m("Toggle Node Mute", "M", "node.mute_toggle"),
        m("Toggle Node Collapse", "H", "node.hide_toggle"),
        m("Toggle Hidden Node Sockets", "Ctrl H", "node.hide_socket_toggle"),
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
            ui.open_menu(
                r,
                "Shader Type",
                vec![
                    MenuItem::op("Object", "", anim::mail("node.shader_type", s.id, "OBJECT")),
                    MenuItem::op("World", "", anim::mail("node.shader_type", s.id, "WORLD")),
                    MenuItem::op("Line Style", "", anim::mail("node.shader_type", s.id, "LINESTYLE")),
                ],
            );
        }
        x = r.right() + 8;
    }
    let (v, se, a, n) = menus(s, d);
    let view = move |_: &Doc| v.clone();
    let sel = move |_: &Doc| se.clone();
    let add = move |_: &Doc| a.clone();
    let node = move |_: &Doc| n.clone();
    let list: [(&str, &dyn Fn(&Doc) -> Vec<MenuItem>); 4] = [("View", &view), ("Select", &sel), ("Add", &add), ("Node", &node)];
    x = header_menus(ui, hr, x, &list, d) + 10;
    match s.kind {
        EditorKind::ShaderEditor if !s.world => {
            let Some(a) = d.active() else { return };
            let n = d.scene.objects[a].materials.len();
            let sr = Rect::new(x, y, 70, h);
            if ui.dropdown(sr, &format!("Slot {}", s.slot.min(n.saturating_sub(1)) + 1)) {
                ui.open_menu(sr, "", (0..n.max(1)).map(|i| MenuItem::op(&format!("Slot {}", i + 1), "", anim::mail("node.slot", s.id, &i.to_string()))).collect());
            }
            x = sr.right() + 6;
            match material_index(s, d) {
                Some(mi) => {
                    let br = Rect::new(x, y, 26, h);
                    if ui.dropdown(br, "") {
                        let items = d.scene.materials.iter().enumerate().map(|(i, m)| MenuItem::op(&m.name, "", anim::mail("node.material_pick", s.id, &i.to_string()))).collect();
                        ui.open_menu(br, "Browse Material", items);
                    }
                    let c = d.scene.materials[mi].base_color;
                    ui.p.circle((br.x + 9) as f32, (br.y + br.h / 2) as f32, 5.0, display([c[0], c[1], c[2]]));
                    let nr = Rect::new(br.right() + 2, y, 140, h);
                    let mut name = d.scene.materials[mi].name.clone();
                    if ui.text_field(id(&format!("node{}/matname", s.id)), nr, &mut name) && !name.trim().is_empty() {
                        d.push_undo("Rename Material");
                        let nm = sex_3d::scene::unique_name(name.trim(), |q| d.scene.materials.iter().enumerate().any(|(i, m)| i != mi && m.name == q));
                        d.scene.materials[mi].name = nm;
                    }
                    x = nr.right() + 8;
                    let mut on = d.scene.materials[mi].node_tree.is_some();
                    let ur = Rect::new(x, y, 90, h);
                    if ui.checkbox(ur, "Use Nodes", &mut on) {
                        d.queue.push(anim::mail("node.use_nodes_toggle", s.id, ""));
                    }
                }
                None => {
                    let nr = Rect::new(x, y, 70, h);
                    if ui.button(nr, "+ New") {
                        d.queue.push(anim::mail("node.material_new", s.id, ""));
                    }
                }
            }
        }
        EditorKind::ShaderEditor => {
            let wr = Rect::new(x, y, 120, h);
            ui.dropdown(wr, "World");
            let mut on = d.scene.world.node_tree.is_some();
            if ui.checkbox(Rect::new(wr.right() + 8, y, 90, h), "Use Nodes", &mut on) {
                d.queue.push(anim::mail("node.use_nodes_toggle", s.id, ""));
            }
        }
        EditorKind::GeometryNodeEditor => {
            let br = Rect::new(x, y, 26, h);
            if ui.dropdown(br, "") {
                let mut items: Vec<MenuItem> =
                    d.scene.node_groups.iter().enumerate().filter(|(_, g)| g.kind == TreeKind::Geometry).map(|(i, g)| MenuItem::op(&g.name, "", anim::mail("node.geo_pick", s.id, &i.to_string()))).collect();
                if items.is_empty() {
                    items.push(MenuItem::Heading("No node groups".into()));
                }
                ui.open_menu(br, "Browse Node Tree", items);
            }
            let nr = Rect::new(br.right() + 2, y, 150, h);
            match s.geo_group.and_then(|g| d.scene.node_groups.get(g)) {
                Some(g) => {
                    ui.p.round_rect(nr, 4, t::BUTTON_DARK);
                    ui.p.label(Rect::new(nr.x + 6, nr.y, nr.w - 8, nr.h), &g.name.clone(), t::TEXT, Align::Left);
                }
                None => {
                    if ui.button(nr, "+ New") {
                        d.queue.push(anim::mail("node.geo_new", s.id, ""));
                    }
                }
            }
        }
        _ => {
            let mut on = d.scene.compositor.is_some();
            if ui.checkbox(Rect::new(x, y, 90, h), "Use Nodes", &mut on) {
                d.queue.push(anim::mail("node.use_nodes_toggle", s.id, ""));
            }
            ui.flat_button(Rect::new(x + 100, y, 70, h), "Backdrop", false);
        }
    }
}

// ---------------------------------------------------------------- operators

/// Nodes that move with `sel`: the selection and everything inside
/// selected frames.
fn moving_set(t: &NodeTree, sel: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = sel.to_vec();
    let mut k = 0;
    while k < v.len() {
        let f = v[k];
        if t.nodes[f].idname == edit::FRAME {
            for (i, n) in t.nodes.iter().enumerate() {
                if n.parent == Some(f) && !v.contains(&i) {
                    v.push(i);
                }
            }
        }
        k += 1;
    }
    v
}

fn start_move(s: &mut State, t: &NodeTree, release_confirms: bool, key: u64) {
    let sel = edit::selected(t);
    if sel.is_empty() {
        return;
    }
    let orig = moving_set(t, &sel).into_iter().map(|i| (i, t.nodes[i].location)).collect();
    s.drag = Drag::Move { x: s.mouse.0, y: s.mouse.1, orig, release_confirms, key };
}

/// Run a node operator on the shown tree (menus and hotkeys arrive here).
pub fn run_op(s: &mut State, d: &mut Doc, op: &str, arg: &str) {
    // Operators that do not need a tree.
    match op {
        "node.shader_type" => {
            if arg == "LINESTYLE" {
                anim::report(d, ReportKind::Warning, "Line Style shaders are not available yet");
            } else {
                s.world = arg == "WORLD";
                s.path.clear();
                s.want_frame = Some(false);
            }
            return;
        }
        "node.sidebar" => {
            s.sidebar = !s.sidebar;
            return;
        }
        "node.slot" => {
            s.slot = arg.parse().unwrap_or(0);
            s.path.clear();
            s.want_frame = Some(false);
            return;
        }
        "node.material_pick" => {
            if let (Ok(mi), Some(a)) = (arg.parse::<usize>(), d.active()) {
                d.push_undo("Assign Material");
                let slot = s.slot;
                let o = &mut d.scene.objects[a];
                if o.materials.is_empty() {
                    o.materials.push(mi);
                } else {
                    let k = slot.min(o.materials.len() - 1);
                    o.materials[k] = mi;
                }
                s.path.clear();
                s.want_frame = Some(false);
                d.touch();
            }
            return;
        }
        "node.material_new" => {
            let Some(a) = d.active() else { return };
            d.push_undo("New Material");
            let name = sex_3d::scene::unique_name("Material", |n| d.scene.materials.iter().any(|m| m.name == n));
            let mut m = Material { name: name.clone(), ..Default::default() };
            m.node_tree = Some(NodeTree::default_material(&name));
            let mi = d.scene.add_material(m);
            let o = &mut d.scene.objects[a];
            if o.materials.is_empty() {
                o.materials.push(mi);
            } else {
                let k = s.slot.min(o.materials.len() - 1);
                o.materials[k] = mi;
            }
            nodetree::after_edit(d, TreeRef::Material(mi));
            s.want_frame = Some(false);
            return;
        }
        "node.geo_new" => {
            d.push_undo("New Node Group");
            let name = sex_3d::scene::unique_name("Geometry Nodes", |n| d.scene.node_groups.iter().any(|g| g.name == n));
            d.scene.node_groups.push(sex_3d::geometry::new_group(&name));
            s.geo_group = Some(d.scene.node_groups.len() - 1);
            // The new group runs on the active mesh object, as a Nodes
            // modifier at the end of its stack.
            if let Some(a) = d.active().filter(|&a| matches!(d.scene.objects[a].data, sex_3d::scene::ObjectData::Mesh(_))) {
                d.scene.objects[a].modifiers.push(sex_3d::modifiers::Modifier::Nodes { group: name, inputs: Vec::new() });
            }
            s.path.clear();
            s.want_frame = Some(false);
            d.touch();
            return;
        }
        "node.geo_pick" => {
            s.geo_group = arg.parse().ok();
            s.path.clear();
            s.want_frame = Some(false);
            return;
        }
        "node.view_all" => {
            s.want_frame = Some(false);
            return;
        }
        "node.view_selected" => {
            s.want_frame = Some(true);
            return;
        }
        "node.add_search" | "node.add_menu" => {
            if tree_ref(s, d).and_then(|r| r.get(&d.scene)).is_some() {
                s.add_menu = Some(addmenu::AddMenu::new(s.mouse.0 - 20, s.mouse.1 - 10, op == "node.add_search" || arg == "search"));
            }
            return;
        }
        "node.select_box" => {
            s.box_armed = true;
            return;
        }
        "node.use_nodes_toggle" => {
            let Some(r) = tree_ref(s, d) else { return };
            if r.get(&d.scene).is_some() {
                // Use Nodes off: the material keeps its look through the
                // legacy fields (Ctrl+Z brings the tree back).
                d.push_undo("Use Nodes");
                match r {
                    TreeRef::Material(mi) => d.scene.materials[mi].node_tree = None,
                    TreeRef::World => d.scene.world.node_tree = None,
                    TreeRef::Compositor => d.scene.compositor = None,
                    TreeRef::Group(_) => {}
                }
                d.touch();
            } else {
                nodetree::use_nodes(d, r);
                s.want_frame = Some(false);
            }
            return;
        }
        _ => {}
    }
    let Some(tref) = tree_ref(s, d) else { return };
    let Some(mut t) = tref.get(&d.scene).cloned() else { return };
    let mut groups = d.scene.node_groups.clone();
    let sel = edit::selected(&t);
    // (undo label, data changed) once the tree changed.
    let mut done: Option<&str> = None;
    let mut select_only = false;
    let mut groups_changed = false;
    match op {
        "node.select_all" => {
            t.nodes.iter_mut().for_each(|n| n.select = true);
            select_only = true;
        }
        "node.select_none" => {
            t.nodes.iter_mut().for_each(|n| n.select = false);
            select_only = true;
        }
        "node.select_invert" => {
            t.nodes.iter_mut().for_each(|n| n.select = !n.select);
            select_only = true;
        }
        "node.select_linked_from" | "node.select_linked_to" => {
            let more = if op == "node.select_linked_from" { edit::linked_from(&t, &sel) } else { edit::linked_to(&t, &sel) };
            for i in more {
                t.nodes[i].select = true;
            }
            select_only = true;
        }
        "node.find_node" => {
            let items = t.nodes.iter().enumerate().map(|(i, n)| MenuItem::op(&if n.label.is_empty() { n.name.clone() } else { format!("{} ({})", n.label, n.name) }, "", anim::mail("node.find", s.id, &i.to_string()))).collect();
            let mut p = crate::ui::Popup::new("Find Node", items, s.mouse.0 - 60, s.mouse.1 - 10);
            p.owner = Rect::default();
            d.menu = Some(p);
            return;
        }
        "node.find" => {
            if let Ok(i) = arg.parse::<usize>() {
                if i < t.nodes.len() {
                    edit::select_only(&mut t, &[i]);
                    t.active = Some(i);
                    s.want_frame = Some(true);
                    select_only = true;
                }
            }
        }
        "node.translate" => {
            let k = s.next_key();
            start_move(s, &t, false, k);
            return;
        }
        "node.add_node" => {
            let mut f = arg.split('\t');
            let idname = f.next().unwrap_or("");
            let at = s.view.to_node(s.mouse.0, s.mouse.1);
            let own = tref.group().and_then(|g| groups.get(g)).cloned();
            let Some(n) = edit::add_node(&mut t, idname, [at.0, at.1], &groups, own.as_ref()) else {
                anim::report(d, ReportKind::Warning, &format!("{idname}: unknown node type"));
                return;
            };
            if let (Some(p), Some(v)) = (f.next(), f.next()) {
                nodetree::set_prop_text(&mut t, n, p, v);
            }
            update::update(&mut t.nodes[n]);
            let w = t.nodes[n].width;
            t.nodes[n].location = [at.0 - w / 2.0, at.1 + 10.0];
            edit::select_only(&mut t, &[n]);
            t.active = Some(n);
            let key = s.next_key();
            d.push_undo("Add Node");
            s.undo_for = Some(key);
            if let Some(dst) = tref.get_mut(&mut d.scene) {
                *dst = t.clone();
            }
            nodetree::after_edit(d, tref);
            start_move(s, &t, false, key);
            return;
        }
        "node.delete" => {
            if !sel.is_empty() {
                edit::remove_nodes(&mut t, &sel);
                done = Some("Delete");
            }
        }
        "node.delete_reconnect" => {
            if !sel.is_empty() {
                edit::delete_reconnect(&mut t, &sel);
                done = Some("Dissolve Nodes");
            }
        }
        "node.duplicate_move" | "node.duplicate_move_keep_inputs" => {
            if !sel.is_empty() {
                edit::duplicate(&mut t, &sel, op.ends_with("keep_inputs"));
                let key = s.next_key();
                d.push_undo("Duplicate Nodes");
                s.undo_for = Some(key);
                if let Some(dst) = tref.get_mut(&mut d.scene) {
                    *dst = t.clone();
                }
                nodetree::after_edit(d, tref);
                start_move(s, &t, false, key);
            }
            return;
        }
        "node.clipboard_copy" => {
            if !sel.is_empty() {
                s.clipboard = Some(edit::copy(&t, &sel));
                d.report = format!("Copied {} node(s)", sel.len());
            }
            return;
        }
        "node.clipboard_paste" => {
            let Some(c) = s.clipboard.clone() else {
                d.report = "Clipboard is empty".into();
                return;
            };
            if c.kind != t.kind {
                anim::report(d, ReportKind::Warning, "Clipboard nodes are from another kind of node tree");
                return;
            }
            // Paste centred at the pointer.
            let at = s.view.to_node(s.mouse.0, s.mouse.1);
            let n = c.nodes.len().max(1) as f32;
            let cx = c.nodes.iter().map(|n| n.location[0] + n.width / 2.0).sum::<f32>() / n;
            let cy = c.nodes.iter().map(|n| n.location[1]).sum::<f32>() / n;
            edit::paste(&mut t, &c, [at.0 - cx, at.1 - cy]);
            done = Some("Paste Nodes");
        }
        "node.mute_toggle" => {
            if !sel.is_empty() {
                for &i in &sel {
                    if t.nodes[i].idname != edit::FRAME {
                        t.nodes[i].mute = !t.nodes[i].mute;
                    }
                }
                done = Some("Toggle Node Mute");
            }
        }
        "node.hide_toggle" => {
            if !sel.is_empty() {
                let hide = !sel.iter().all(|&i| t.nodes[i].hide);
                for &i in &sel {
                    t.nodes[i].hide = hide;
                }
                done = Some("Toggle Node Collapse");
            }
        }
        "node.hide_socket_toggle" => {
            if !sel.is_empty() {
                // Hide unused sockets if any is shown, else show them all.
                let mut any_shown = false;
                for &i in &sel {
                    let n = &t.nodes[i];
                    for (k, so) in n.inputs.iter().enumerate() {
                        any_shown |= !so.hide && !layout::is_linked(&t, i, k) && so.ty != SocketType::Virtual;
                    }
                    for (k, so) in n.outputs.iter().enumerate() {
                        any_shown |= !so.hide && !t.links.iter().any(|l| l.from_node == i && l.from_socket == k) && so.ty != SocketType::Virtual;
                    }
                }
                for &i in &sel {
                    for k in 0..t.nodes[i].inputs.len() {
                        let used = layout::is_linked(&t, i, k);
                        t.nodes[i].inputs[k].hide = any_shown && !used;
                    }
                    for k in 0..t.nodes[i].outputs.len() {
                        let used = t.links.iter().any(|l| l.from_node == i && l.from_socket == k);
                        t.nodes[i].outputs[k].hide = any_shown && !used;
                    }
                }
                done = Some("Toggle Hidden Node Sockets");
            }
        }
        "node.link_make" => {
            if edit::make_links(&mut t, &sel, arg == "REPLACE") > 0 {
                done = Some("Make Links");
            }
        }
        "node.links_cut" => {
            d.report = "Ctrl+right-drag across links to cut them".into();
            return;
        }
        "node.join" => {
            if !sel.is_empty() {
                let f = edit::join_frame(&mut t, &sel);
                edit::select_only(&mut t, &[f]);
                t.active = Some(f);
                done = Some("Join Nodes");
            }
        }
        "node.detach" => {
            if !sel.is_empty() {
                edit::detach(&mut t, &sel);
                done = Some("Detach Nodes");
            }
        }
        "node.group_make" => {
            let gi = groups.len();
            if edit::make_group(&mut t, &sel, &mut groups).is_some() {
                groups[gi].name = sex_3d::scene::unique_name("NodeGroup", |n| d.scene.node_groups.iter().any(|g| g.name == n));
                groups_changed = true;
                done = Some("Make Group");
            }
        }
        "node.group_ungroup" => {
            let gnodes: Vec<usize> = sel.iter().copied().filter(|&i| edit::group_of(&t.nodes[i]).is_some()).collect();
            for &g in gnodes.iter().rev() {
                edit::ungroup(&mut t, g, &groups);
            }
            if !gnodes.is_empty() {
                done = Some("Ungroup");
            }
        }
        "node.group_edit" => {
            let target = t.active.filter(|&a| t.nodes.get(a).is_some_and(|n| n.select)).and_then(|a| edit::group_of(&t.nodes[a]));
            match target {
                Some(g) if g < groups.len() => s.path.push(g),
                _ => {
                    s.path.pop();
                }
            }
            s.want_frame = Some(false);
            s.add_menu = None;
            return;
        }
        "node.set_socket_id" => {
            let f: Vec<usize> = arg.split('\t').filter_map(|x| x.parse().ok()).collect();
            if f.len() >= 2 {
                if let Some(so) = t.nodes.get_mut(f[0]).and_then(|n| n.inputs.get_mut(f[1])) {
                    so.value = Value::Id(f.get(2).copied());
                    done = Some("Change Node Value");
                }
            }
        }
        "node.ramp_interp" => {
            let f: Vec<&str> = arg.split('\t').collect();
            if let (Some(n), Some(p), Some(v)) = (f.first().and_then(|x| x.parse::<usize>().ok()), f.get(1).and_then(|x| x.parse::<usize>().ok()), f.get(2)) {
                if let Some((_, Prop::Ramp(r))) = t.nodes.get_mut(n).and_then(|n| n.props.get_mut(p)) {
                    r.interpolation = v.to_string();
                    done = Some("Color Ramp Interpolation");
                }
            }
        }
        "node.image_open" => {
            if let Ok(n) = arg.parse::<usize>() {
                s.open_path = Some((n, String::new()));
            }
            return;
        }
        "node.interface_add" | "node.interface_remove" | "node.interface_move" => {
            let Some(g) = tref.group() else { return };
            interface_op(s, d, g, op, arg);
            return;
        }
        _ => {
            d.report = format!("{op} is not available yet");
            return;
        }
    }
    if select_only {
        if let Some(dst) = tref.get_mut(&mut d.scene) {
            *dst = t;
        }
        return;
    }
    if let Some(label) = done {
        d.push_undo(label);
        if groups_changed {
            d.scene.node_groups = groups;
        }
        if let Some(dst) = tref.get_mut(&mut d.scene) {
            *dst = t;
        }
        nodetree::after_edit(d, tref);
    }
}

/// Group interface edits (sidebar's Group tab).
fn interface_op(s: &mut State, d: &mut Doc, g: usize, op: &str, arg: &str) {
    let f: Vec<&str> = arg.split('\t').collect();
    let input = f.first() == Some(&"in");
    let idx: Option<usize> = f.get(1).and_then(|x| x.parse().ok());
    let len = {
        let gr = &d.scene.node_groups[g];
        if input {
            gr.inputs.len()
        } else {
            gr.outputs.len()
        }
    };
    match op {
        "node.interface_add" => {
            d.push_undo("Add Interface Socket");
            let gr = &mut d.scene.node_groups[g];
            let ty = if gr.kind == TreeKind::Geometry { SocketType::Geometry } else if input { SocketType::Float } else { SocketType::Color };
            let name = if input { "Input" } else { "Output" };
            let sock = sex_3d::nodes::InterfaceSocket { name: name.into(), ty, default: edit::type_default(ty), min: f32::MIN, max: f32::MAX };
            if input {
                gr.inputs.push(sock);
            } else {
                gr.outputs.push(sock);
            }
            // The extend socket moves one down.
            nodetree::interface_changed(&mut d.scene, g, input, &|i| Some(if i >= len { i + 1 } else { i }));
            s.iface_sel = Some((input, len));
        }
        "node.interface_remove" => {
            let Some(i) = idx.filter(|&i| i < len) else { return };
            d.push_undo("Remove Interface Socket");
            let gr = &mut d.scene.node_groups[g];
            if input {
                gr.inputs.remove(i);
            } else {
                gr.outputs.remove(i);
            }
            nodetree::interface_changed(&mut d.scene, g, input, &|k| if k == i { None } else if k > i { Some(k - 1) } else { Some(k) });
            s.iface_sel = if len > 1 { Some((input, i.min(len - 2))) } else { None };
        }
        _ => {
            let Some(i) = idx.filter(|&i| i < len) else { return };
            let up = f.get(2) == Some(&"-1");
            let j = if up { i.checked_sub(1) } else { (i + 1 < len).then_some(i + 1) };
            let Some(j) = j else { return };
            d.push_undo("Move Interface Socket");
            let gr = &mut d.scene.node_groups[g];
            if input {
                gr.inputs.swap(i, j);
            } else {
                gr.outputs.swap(i, j);
            }
            nodetree::interface_changed(&mut d.scene, g, input, &|k| Some(if k == i { j } else if k == j { i } else { k }));
            s.iface_sel = Some((input, j));
        }
    }
    d.touch();
}

// ---------------------------------------------------------------- body

#[derive(Default)]
struct Ed {
    /// (undo label, undo key) when the tree's data changed.
    data: Option<(String, u64)>,
    /// Only the selection changed.
    select: bool,
    /// Reload the tree from the document (an edit went there directly).
    reload: bool,
}

impl Ed {
    fn change(&mut self, label: &str, key: u64) {
        if self.data.is_none() {
            self.data = Some((label.to_string(), key));
        }
    }
}

fn commit(s: &mut State, d: &mut Doc, tref: TreeRef, t: NodeTree, ed: &Ed) {
    if let Some((label, key)) = &ed.data {
        if s.undo_for != Some(*key) {
            d.push_undo(label);
            s.undo_for = Some(*key);
        }
        if let Some(dst) = tref.get_mut(&mut d.scene) {
            *dst = t;
        }
        nodetree::after_edit(d, tref);
    } else if ed.select {
        if let Some(dst) = tref.get_mut(&mut d.scene) {
            *dst = t;
        }
    }
}

/// Draw order: frames (outer first), plain nodes, selected nodes, the active one.
fn draw_order(t: &NodeTree) -> Vec<usize> {
    let depth = |mut i: usize| {
        let mut k = 0;
        while let Some(p) = t.nodes[i].parent {
            k += 1;
            i = p;
            if k > t.nodes.len() {
                break;
            }
        }
        k
    };
    let mut frames: Vec<usize> = (0..t.nodes.len()).filter(|&i| t.nodes[i].idname == edit::FRAME).collect();
    frames.sort_by_key(|&f| depth(f));
    let mut rest: Vec<usize> = (0..t.nodes.len()).filter(|&i| t.nodes[i].idname != edit::FRAME).collect();
    rest.sort_by_key(|&i| (t.nodes[i].select as u8) + 2 * (t.active == Some(i)) as u8);
    frames.extend(rest);
    frames
}

fn node_at(t: &NodeTree, lay: &[NodeLayout], order: &[usize], x: i32, y: i32) -> Option<usize> {
    order.iter().rev().copied().find(|&i| {
        let r = lay[i].rect;
        if t.nodes[i].idname == edit::REROUTE {
            let (cx, cy) = r.center();
            return (x - cx).pow(2) + (y - cy).pow(2) <= (r.w / 2 + 4).pow(2);
        }
        r.contains(x, y)
    })
}

fn frame_view(s: &mut State, t: &NodeTree, lay: &[NodeLayout], only_selected: bool) {
    let v = s.view;
    let (mut lo, mut hi) = ((f32::INFINITY, f32::INFINITY), (f32::NEG_INFINITY, f32::NEG_INFINITY));
    for (i, l) in lay.iter().enumerate() {
        if only_selected && !t.nodes[i].select {
            continue;
        }
        let a = v.to_node(l.rect.x, l.rect.y);
        let b = v.to_node(l.rect.right(), l.rect.bottom());
        lo = (lo.0.min(a.0.min(b.0)), lo.1.min(a.1.min(b.1)));
        hi = (hi.0.max(a.0.max(b.0)), hi.1.max(a.1.max(b.1)));
    }
    if !lo.0.is_finite() {
        if !only_selected {
            s.center = (0.0, 0.0);
            s.zoom = 1.0;
        }
        return;
    }
    s.center = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
    let r = v.region;
    let zx = r.w as f32 * 0.85 / (hi.0 - lo.0).max(1.0);
    let zy = r.h as f32 * 0.85 / (hi.1 - lo.1).max(1.0);
    s.zoom = zx.min(zy).clamp(0.25, 1.2);
}

fn draw_grid(s: &State, p: &mut Painter, r: Rect) {
    let mut step = 20.0 * s.zoom;
    while step < 12.0 {
        step *= 2.0;
    }
    let (ox, oy) = s.view.to_screen((0.0, 0.0));
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

fn empty_state(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect, tref: Option<TreeRef>) {
    let (cx, cy) = r.center();
    let b = Rect::new(cx - 60, cy - 12, 120, 24);
    match (s.kind, tref) {
        (EditorKind::ShaderEditor, None) if !s.world => {
            if d.active().is_some() {
                if ui.button(b, "+ New Material") {
                    d.queue.push(anim::mail("node.material_new", s.id, ""));
                }
            } else {
                ui.p.label(r, "No active object", t::TEXT_DIM, Align::Center);
            }
        }
        (EditorKind::GeometryNodeEditor, None) => {
            if ui.button(b, "+ New") {
                d.queue.push(anim::mail("node.geo_new", s.id, ""));
            }
        }
        (_, Some(_)) => {
            if ui.button(b, "Use Nodes") {
                d.queue.push(anim::mail("node.use_nodes_toggle", s.id, ""));
            }
        }
        _ => {}
    }
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    for (op, arg) in anim::take_mail(d, s.id) {
        run_op(s, d, &op, &arg);
    }
    s.path.retain(|&g| g < d.scene.node_groups.len());
    if let Some(p) = ui.input.and_then(|i| i.pos()) {
        s.mouse = p;
    }
    ui.p.fill(r, t::NODE_BG);
    let (side, canvas) = if s.sidebar && r.w > SIDEBAR_W + 100 { r.take_right(SIDEBAR_W) } else { (Rect::default(), r) };
    s.view = View { center: s.center, zoom: s.zoom, region: canvas };
    draw_grid(s, ui.p, canvas);
    s.fields.clear();
    let tref = tree_ref(s, d);
    let Some(mut tr) = tref.and_then(|r| r.get(&d.scene)).cloned() else {
        s.layout.clear();
        s.add_menu = None;
        s.drag = Drag::None;
        empty_state(s, d, ui, canvas, tref);
        return;
    };
    let tref = tref.unwrap();

    // The colour picker sits on top: it sees events first.
    let mut stash = None;
    if let Some(pk) = &s.picker {
        match picker::claims(pk, ui) {
            picker::Claim::Mine => stash = ui.input.take(),
            picker::Claim::Close => s.picker = None,
            picker::Claim::No => {}
        }
    }
    // The Add menu takes everything while open.
    let current_group = tref.group();
    let cats = addmenu::categories(tr.kind, &d.scene.node_groups, current_group);
    if let Some(mut m) = s.add_menu.take() {
        match m.handle(ui, &cats, canvas) {
            addmenu::Outcome::Open => s.add_menu = Some(m),
            addmenu::Outcome::Close => {}
            addmenu::Outcome::Pick(e) => {
                run_op(s, d, "node.add_node", &e.arg());
                match tref.get(&d.scene) {
                    Some(t2) => tr = t2.clone(),
                    None => return,
                }
            }
        }
    }
    // Events over the sidebar are its own.
    let side_input = match ui.input.and_then(|i| i.pos()) {
        Some((x, y)) if side.contains(x, y) && matches!(s.drag, Drag::None) => ui.input.take(),
        _ => None,
    };

    let mut t = tr;
    let mut ed = Ed::default();
    let lay = layout::layout(&t, &s.view, &s.open_panels);
    if let Some(sel_only) = s.want_frame.take() {
        if canvas.w > 0 {
            frame_view(s, &t, &lay, sel_only);
            s.view = View { center: s.center, zoom: s.zoom, region: canvas };
        }
    }
    let lay = layout::layout(&t, &s.view, &s.open_panels);
    modal(s, d, ui, &mut t, &lay, &mut ed, tref);
    if ed.reload {
        if let Some(t2) = tref.get(&d.scene) {
            t = t2.clone();
        }
        ed = Ed::default();
    }
    s.view = View { center: s.center, zoom: s.zoom, region: canvas };
    let lay = layout::layout(&t, &s.view, &s.open_panels);
    let order = draw_order(&t);

    ui.p.push_clip(canvas);
    draw_links(s, ui.p, &t, &lay);
    // Only the topmost node under a click may take it.
    let top = match ui.input {
        Some(Input::Down { x, y, .. }) => Some(node_at(&t, &lay, &order, x, y)),
        _ => None,
    };
    for &n in &order {
        let withhold = matches!(top, Some(tp) if tp != Some(n));
        let saved = if withhold { ui.input.take() } else { None };
        draw_node(s, d, ui, &mut t, n, &lay, &mut ed, tref);
        if withhold {
            ui.input = saved;
        }
    }
    canvas_input(s, d, ui, &mut t, &lay, &order, &mut ed, canvas);
    // Overlays.
    match &s.drag {
        Drag::Box { x0, y0, x1, y1, .. } => {
            let b = Rect::new(*x0.min(x1), *y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
            ui.p.fill_alpha(b, 0xffffff, 18);
            ui.p.frame(b, 0xbbbbbb);
        }
        Drag::Link { from, to, x, y, .. } => {
            let (a, b) = match (from, to) {
                (Some((n, o)), _) => (lay.get(*n).and_then(|l| l.outputs.get(*o).copied().flatten()), Some((*x as f32, *y as f32))),
                (None, Some((n, i))) => (Some((*x as f32, *y as f32)), lay.get(*n).and_then(|l| l.inputs.get(*i).copied().flatten())),
                _ => (None, None),
            };
            if let (Some(a), Some(b)) = (a, b) {
                link_curve(ui.p, a, b, s.zoom, 0xffffff);
            }
        }
        Drag::Knife(pts) | Drag::Reroute(pts) => {
            let col = if matches!(s.drag, Drag::Knife(_)) { 0xff5050 } else { 0x50ff80 };
            for w in pts.windows(2) {
                ui.p.line(w[0].0 as f32, w[0].1 as f32, w[1].0 as f32, w[1].1 as f32, col);
            }
        }
        _ => {}
    }
    // Where we are: material or group path.
    let mut crumbs = match tref {
        TreeRef::Material(mi) => d.scene.materials.get(mi).map(|m| m.name.clone()).unwrap_or_default(),
        TreeRef::World => "World".into(),
        TreeRef::Compositor => "Compositing".into(),
        TreeRef::Group(g) => d.scene.node_groups.get(g).map(|x| x.name.clone()).unwrap_or_default(),
    };
    if !s.path.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        if let Some(base) = {
            let mut s2 = State::new(s.kind);
            s2.world = s.world;
            s2.slot = s.slot;
            s2.geo_group = s.geo_group;
            tree_ref(&s2, d)
        } {
            parts.push(match base {
                TreeRef::Material(mi) => d.scene.materials.get(mi).map(|m| m.name.clone()).unwrap_or_default(),
                TreeRef::World => "World".into(),
                TreeRef::Compositor => "Compositing".into(),
                TreeRef::Group(g) => d.scene.node_groups.get(g).map(|x| x.name.clone()).unwrap_or_default(),
            });
        }
        for &g in &s.path {
            parts.push(d.scene.node_groups.get(g).map(|x| x.name.clone()).unwrap_or_default());
        }
        crumbs = parts.join("  \u{25B8}  ");
    }
    ui.p.label(Rect::new(canvas.x + 12, canvas.y + 6, canvas.w - 24, 18), &crumbs, t::TEXT_DIM, Align::Left);
    ui.p.pop_clip();

    commit(s, d, tref, t, &ed);
    s.layout = layout::layout(tref.get(&d.scene).unwrap_or(&NodeTree::default()), &s.view, &s.open_panels);

    if !side.is_empty() {
        let saved = ui.input.take();
        ui.input = side_input;
        sidebar::draw(s, d, ui, side, tref);
        if ui.input.is_none() {
            // The sidebar used it.
        } else {
            ui.input = saved.or(ui.input);
        }
    }
    if let Some(m) = &s.add_menu {
        ui.p.push_clip(canvas);
        m.draw(ui, &cats, canvas);
        ui.p.pop_clip();
    }
    if stash.is_some() {
        ui.input = stash;
    }
    if let Some(pk) = &mut s.picker {
        if !picker::draw(pk, d, ui, r, &mut s.picker_undo) {
            s.picker = None;
        }
    }
    if ui.st.active.is_none() && matches!(s.drag, Drag::None) && ui.st.editing.is_none() {
        s.undo_for = None;
    }
}

// ---------------------------------------------------------------- drawing

fn link_curve(p: &mut Painter, a: (f32, f32), b: (f32, f32), zoom: f32, col: u32) {
    let pts = layout::link_points(a, b, zoom, 28);
    for w in pts.windows(2) {
        let (q, r) = (w[0], w[1]);
        p.line(q.0, q.1 + 1.2, r.0, r.1 + 1.2, 0x101010);
        p.line(q.0, q.1 - 0.5, r.0, r.1 - 0.5, col);
        p.line(q.0, q.1 + 0.5, r.0, r.1 + 0.5, col);
    }
}

fn draw_links(s: &State, p: &mut Painter, t: &NodeTree, lay: &[NodeLayout]) {
    for l in &t.links {
        let (Some(a), Some(b)) = (lay.get(l.from_node).and_then(|x| x.outputs.get(l.from_socket).copied().flatten()), lay.get(l.to_node).and_then(|x| x.inputs.get(l.to_socket).copied().flatten())) else { continue };
        let ok = edit::can_link(t.nodes[l.from_node].outputs[l.from_socket].ty, t.nodes[l.to_node].inputs[l.to_socket].ty);
        let col = if !ok {
            0xd04040
        } else if l.muted {
            0x804040
        } else if t.nodes[l.from_node].select || t.nodes[l.to_node].select {
            0xf0f0f0
        } else {
            0xa8a8a8
        };
        link_curve(p, a, b, s.zoom, col);
    }
}

fn socket_dot(p: &mut Painter, x: f32, y: f32, ty: SocketType, zoom: f32) {
    let r = (4.5 * zoom).max(2.5);
    if ty == SocketType::Virtual {
        p.ring(x, y, r, 0x9a9a9a);
        return;
    }
    p.circle(x, y, r + 1.0, 0x000000);
    p.circle(x, y, r, ty.color());
}

#[allow(clippy::too_many_arguments)]
fn draw_node(s: &mut State, d: &mut Doc, ui: &mut Ui, t: &mut NodeTree, n: usize, lay: &[NodeLayout], ed: &mut Ed, tref: TreeRef) {
    let l = &lay[n];
    let r = l.rect;
    let z = s.zoom;
    let node = t.nodes[n].clone();
    let selected = node.select;
    let active = t.active == Some(n);
    let outline = if active && selected { Some(0xffffff) } else if selected { Some(0xed5700) } else { None };
    if !ui.p.clip().intersect(&Rect::new(r.x - 20, r.y - 20, r.w + 40, r.h + 40)).is_empty() {
    } else {
        return;
    }
    if node.idname == edit::FRAME {
        let c = node.color.map(display).unwrap_or(0x4d4d4d);
        ui.p.round_rect_alpha(r, (4.0 * z) as i32, c, 110);
        if let Some(o) = outline {
            ui.p.round_frame(r, (4.0 * z) as i32, o);
        }
        if z >= 0.4 && !node.label.is_empty() {
            ui.p.label(Rect::new(r.x + 8, r.y + 2, r.w - 16, (layout::FRAME_HEAD * z) as i32), &node.label, t::TEXT, Align::Left);
        }
        return;
    }
    if node.idname == edit::REROUTE {
        let (cx, cy) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
        let ty = t.links.iter().find(|k| k.to_node == n).map(|k| t.nodes[k.from_node].outputs[k.from_socket].ty).unwrap_or(SocketType::Color);
        if let Some(o) = outline {
            ui.p.circle(cx, cy, 6.5 * z.max(0.6), o);
        }
        socket_dot(ui.p, cx, cy, ty, z);
        return;
    }
    let rad = (5.0 * z).round().max(2.0) as i32;
    ui.p.round_rect_alpha(Rect::new(r.x + 2, r.y + 3, r.w, r.h), rad, 0x000000, 80);
    if let Some(c) = outline {
        ui.p.round_rect(Rect::new(r.x - 2, r.y - 2, r.w + 4, r.h + 4), rad + 2, c);
    }
    let body_col = node.color.map(display).unwrap_or(NODE_BODY);
    ui.p.round_rect(r, rad, body_col);
    let hh = ((HEAD * z).round() as i32).min(r.h);
    let head_col = if node.mute { 0x5a5a5a } else { layout::header_color(&node) };
    ui.p.round_rect(Rect::new(r.x, r.y, r.w, if node.hide { r.h } else { hh + rad }), rad, head_col);
    if !node.hide {
        ui.p.fill(Rect::new(r.x, r.y + hh, r.w, rad + 1), body_col);
    }
    let text = z >= 0.5;
    let arrow = Rect::new(r.x, r.y, (22.0 * z) as i32, hh);
    if text {
        ui.p.arrow(r.x as f32 + 11.0 * z, r.y as f32 + hh as f32 / 2.0, 7.0, !node.hide, 0xdddddd);
        let label = match edit::group_of(&node) {
            Some(g) => d.scene.node_groups.get(g).map(|g| g.name.clone()).unwrap_or_else(|| node.display_label()),
            None => node.display_label(),
        };
        ui.p.push_clip(Rect::new(r.x, r.y, r.w - 4, hh));
        ui.p.label(Rect::new(r.x + (20.0 * z) as i32, r.y, r.w - (24.0 * z) as i32, hh), &label, 0xeeeeee, Align::Left);
        ui.p.pop_clip();
    }
    if ui.clicked(arrow) {
        t.nodes[n].hide = !node.hide;
        ed.change("Toggle Node Collapse", id(&format!("node{}/hide/{}", s.id, node.name)));
    }
    // Rows.
    ui.p.push_clip(Rect::new(r.x - 10, r.y, r.w + 20, r.h));
    if text && !node.hide {
        let rows = l.rows.clone();
        for (rr, kind) in rows {
            draw_row(s, d, ui, t, n, rr, &kind, ed, tref);
        }
    }
    ui.p.pop_clip();
    // Sockets on top.
    for (i, p) in l.inputs.iter().enumerate() {
        if let Some(p) = p {
            socket_dot(ui.p, p.0, p.1, node.inputs[i].ty, z);
        }
    }
    for (i, p) in l.outputs.iter().enumerate() {
        if let Some(p) = p {
            socket_dot(ui.p, p.0, p.1, node.outputs[i].ty, z);
        }
    }
    if node.mute {
        ui.p.round_rect_alpha(r, rad, 0x1d1d1d, 90);
        for (i, o) in edit::internal_links(t, n) {
            if let (Some(Some(a)), Some(Some(b))) = (l.inputs.get(i), l.outputs.get(o)) {
                ui.p.line(a.0, a.1, b.0, b.1, 0xc04040);
                ui.p.line(a.0, a.1 + 1.0, b.0, b.1 + 1.0, 0xc04040);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_row(s: &mut State, d: &mut Doc, ui: &mut Ui, t: &mut NodeTree, n: usize, rr: Rect, kind: &RowKind, ed: &mut Ed, tref: TreeRef) {
    let z = s.zoom;
    let pad = (8.0 * z).round() as i32;
    let fh = ((ROW - 3.0) * z).round() as i32;
    let w = Rect::new(rr.x + pad, rr.y + 1, rr.w - 2 * pad, fh);
    let node_name = t.nodes[n].name.clone();
    let wid = |k: &str| id(&format!("node{}/{}/{}/{k}", s.id, tref.encode(), node_name));
    let row_px = (ROW * z).round() as i32;
    match kind {
        RowKind::Output(i) => {
            let so = &t.nodes[n].outputs[*i];
            ui.p.label(Rect::new(rr.x + pad, rr.y, rr.w - 2 * pad, row_px), &so.name.clone(), t::TEXT, Align::Right);
        }
        RowKind::OutputValue(i) => {
            let so = t.nodes[n].outputs[*i].clone();
            s.fields.push((format!("{node_name}/{}", so.name), w));
            match so.value {
                Value::Color(c) => {
                    let sw = Rect::new(w.x, w.y, w.w, rr.h - 4);
                    if ui.swatch(sw, [c[0], c[1], c[2]]) {
                        s.picker = Some(Picker::new(ColorTarget::Node(tref, n, NodeColor::Output(*i)), sw, d));
                        s.picker_undo = None;
                    }
                }
                Value::Float(f) => {
                    let mut v = f;
                    let k = wid("outval");
                    if ui.number_ex(k, w, "", &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
                        t.nodes[n].outputs[*i].value = Value::Float(v);
                        ed.change("Change Node Value", k);
                    }
                }
                _ => {}
            }
        }
        RowKind::Panel(p) => {
            ui.p.fill(Rect::new(rr.x + 1, rr.y, rr.w - 2, 1), 0x262626);
            let key = (node_name.clone(), p.clone());
            let open = s.open_panels.contains(&key);
            ui.p.arrow(w.x as f32 + 5.0, rr.y as f32 + row_px as f32 / 2.0, 7.0, open, t::TEXT_DIM);
            ui.p.label(Rect::new(w.x + 14, rr.y, w.w - 14, row_px), p, t::TEXT, Align::Left);
            if ui.clicked(w) {
                if open {
                    s.open_panels.remove(&key);
                } else {
                    s.open_panels.insert(key);
                }
            }
        }
        RowKind::Prop(k) => draw_prop(s, d, ui, t, n, *k, rr, w, ed, tref),
        RowKind::Input(i) => {
            let so = t.nodes[n].inputs[*i].clone();
            let linked = layout::is_linked(t, n, *i);
            let label_only = linked || so.hide_value || !layout::has_value(so.ty);
            let lr = Rect::new(rr.x + pad, rr.y, rr.w - 2 * pad, row_px);
            if label_only || (so.ty == SocketType::Vector && matches!(so.value, Value::Vector(_))) {
                ui.p.label(lr, &so.name, t::TEXT, Align::Left);
                return;
            }
            s.fields.push((format!("{node_name}/{}", so.name), w));
            let k = wid(&format!("in/{}", so.identifier));
            match so.value.clone() {
                Value::Float(f) => {
                    let (min, max, factor) = layout::float_range(&so);
                    let angle = so.subtype == Subtype::Angle;
                    let mut v = if angle { f.to_degrees() } else { f };
                    let o = NumOpts {
                        step: if factor { (max - min) / 100.0 } else { 0.01 },
                        min: if angle { f32::NEG_INFINITY } else { min },
                        max: if angle { f32::INFINITY } else { max },
                        slider: factor,
                        unit: if angle { "°" } else { "" },
                        ..NumOpts::default()
                    };
                    if ui.number_ex(k, w, &so.name, &mut v, o) {
                        t.nodes[n].inputs[*i].value = Value::Float(if angle { v.to_radians() } else { v });
                        ed.change("Change Node Value", k);
                    }
                }
                Value::Int(x) => {
                    let mut v = x as f32;
                    let o = NumOpts { step: 1.0, int: true, min: if so.min <= f32::MIN { f32::NEG_INFINITY } else { so.min }, max: if so.max >= f32::MAX { f32::INFINITY } else { so.max }, ..NumOpts::default() };
                    if ui.number_ex(k, w, &so.name, &mut v, o) {
                        t.nodes[n].inputs[*i].value = Value::Int(v as i32);
                        ed.change("Change Node Value", k);
                    }
                }
                Value::Bool(b) => {
                    let mut v = b;
                    if ui.checkbox(w, &so.name, &mut v) {
                        t.nodes[n].inputs[*i].value = Value::Bool(v);
                        ed.change("Change Node Value", k);
                    }
                }
                Value::Color(c) => {
                    let half = w.w / 2;
                    ui.p.label(Rect::new(w.x, w.y, half - 4, w.h), &so.name, t::TEXT, Align::Left);
                    let sw = Rect::new(w.x + half, w.y, w.w - half, w.h);
                    if ui.swatch(sw, [c[0], c[1], c[2]]) {
                        s.picker = Some(Picker::new(ColorTarget::Node(tref, n, NodeColor::Input(*i)), sw, d));
                        s.picker_undo = None;
                    }
                }
                Value::String(st) => {
                    let mut v = st;
                    if ui.text_field(k, w, &mut v) {
                        t.nodes[n].inputs[*i].value = Value::String(v);
                        ed.change("Change Node Value", k);
                    }
                }
                Value::Id(v) => {
                    let (list, prop): (Vec<String>, &str) = match so.ty {
                        SocketType::Image => (d.scene.images.iter().map(|x| x.name.clone()).collect(), "image"),
                        SocketType::Object => (d.scene.objects.iter().map(|x| x.name.clone()).collect(), "object"),
                        SocketType::Material => (d.scene.materials.iter().map(|x| x.name.clone()).collect(), "material"),
                        SocketType::Collection => (d.scene.collections.iter().map(|x| x.name.clone()).collect(), "collection"),
                        _ => (Vec::new(), ""),
                    };
                    let _ = prop;
                    let shown = v.and_then(|i| list.get(i).cloned()).unwrap_or_else(|| so.name.clone());
                    if ui.dropdown(w, &shown) {
                        let mut items: Vec<MenuItem> = list.iter().enumerate().map(|(j, nm)| MenuItem::op(nm, "", anim::mail("node.set_socket_id", s.id, &format!("{n}\t{i}\t{j}")))).collect();
                        items.push(MenuItem::op("None", "", anim::mail("node.set_socket_id", s.id, &format!("{n}\t{i}"))));
                        ui.open_menu(w, &so.name, items);
                    }
                }
                _ => ui.p.label(lr, &so.name, t::TEXT, Align::Left),
            }
        }
        RowKind::VectorComp(i, c) => {
            let so = t.nodes[n].inputs[*i].clone();
            let Value::Vector(vv) = so.value else { return };
            let k = wid(&format!("in/{}/{c}", so.identifier));
            let mut v = vv[*c];
            let unit = if so.subtype == Subtype::Euler || so.subtype == Subtype::Angle { "°" } else { "" };
            if unit == "°" {
                v = v.to_degrees();
            }
            s.fields.push((format!("{node_name}/{}/{}", so.name, ["X", "Y", "Z"][*c]), w));
            if ui.number_ex(k, w, ["X", "Y", "Z"][*c], &mut v, NumOpts { step: 0.01, unit, ..NumOpts::default() }) {
                let mut nv = vv;
                nv[*c] = if unit == "°" { v.to_radians() } else { v };
                t.nodes[n].inputs[*i].value = Value::Vector(nv);
                ed.change("Change Node Value", k);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_prop(s: &mut State, d: &mut Doc, ui: &mut Ui, t: &mut NodeTree, n: usize, k: usize, rr: Rect, w: Rect, ed: &mut Ed, tref: TreeRef) {
    let node = t.nodes[n].clone();
    let (name, p) = node.props[k].clone();
    let key = id(&format!("node{}/{}/{}/p/{name}", s.id, tref.encode(), node.name));
    let label = enums::prop_label(&name);
    s.fields.push((format!("{}/{name}", node.name), w));
    let row = (ROW * s.zoom).round() as i32;
    match p {
        Prop::Enum(v) => {
            if ui.dropdown(w, enums::label(&node.idname, &name, &v)) {
                let mut items: Vec<MenuItem> =
                    enums::items(&node.idname, &name).iter().map(|(i, l)| MenuItem::op(l, "", Op::with("node.set_prop", &format!("{}\t{n}\t{name}\t{i}", tref.encode())))).collect();
                if items.is_empty() {
                    items.push(MenuItem::Heading(v.clone()));
                }
                ui.open_menu(w, &label, items);
            }
        }
        Prop::Bool(b) => {
            let mut v = b;
            if ui.checkbox(w, &label, &mut v) {
                t.nodes[n].props[k].1 = Prop::Bool(v);
                update::update(&mut t.nodes[n]);
                ed.change(&label, key);
            }
        }
        Prop::Int(x) => {
            let mut v = x as f32;
            if ui.number_ex(key, w, &label, &mut v, NumOpts { step: 1.0, int: true, ..NumOpts::default() }) {
                t.nodes[n].props[k].1 = Prop::Int(v as i32);
                ed.change(&label, key);
            }
        }
        Prop::Float(x) => {
            let mut v = x;
            if ui.number_ex(key, w, &label, &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
                t.nodes[n].props[k].1 = Prop::Float(v);
                ed.change(&label, key);
            }
        }
        Prop::String(st) => {
            let mut v = st;
            if ui.text_field(key, w, &mut v) {
                t.nodes[n].props[k].1 = Prop::String(v);
                ed.change(&label, key);
            }
        }
        Prop::Color(c) => {
            if ui.swatch(w, [c[0], c[1], c[2]]) {
                s.picker = Some(Picker::new(ColorTarget::Node(tref, n, NodeColor::Prop(k)), w, d));
                s.picker_undo = None;
            }
        }
        Prop::Vector(vv) => {
            for c in 0..3 {
                let fr = Rect::new(w.x, rr.y + 1 + c as i32 * row, w.w, w.h);
                let mut v = vv[c];
                if ui.number_ex(id(&format!("{key}/{c}")), fr, ["X", "Y", "Z"][c], &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
                    let mut nv = vv;
                    nv[c] = v;
                    t.nodes[n].props[k].1 = Prop::Vector(nv);
                    ed.change(&label, key);
                }
            }
        }
        Prop::Id(v) => {
            let (names, what): (Vec<String>, &str) = match name.as_str() {
                "image" => (d.scene.images.iter().map(|x| x.name.clone()).collect(), "Image"),
                "object" => (d.scene.objects.iter().map(|x| x.name.clone()).collect(), "Object"),
                "node_tree" => (d.scene.node_groups.iter().map(|x| x.name.clone()).collect(), "Node Group"),
                _ => (Vec::new(), "Data"),
            };
            if name == "image" {
                if let Some((on, text)) = s.open_path.clone() {
                    if on == n {
                        let mut tx = text;
                        let fid = id(&format!("node{}/openpath", s.id));
                        if ui.st.editing.as_ref().map(|e| e.0) != Some(fid) && s.open_path.as_ref().is_some_and(|o| o.1.is_empty()) {
                            ui.st.editing = Some((fid, String::new()));
                            ui.st.edit_select_all = false;
                        }
                        ui.p.label(Rect::new(w.x, w.y - row, w.w, row), "Image path:", t::TEXT_DIM, Align::Left);
                        let entered = ui.text_field(fid, w, &mut tx);
                        if entered || ui.st.editing.is_none() {
                            s.open_path = None;
                            if !tx.trim().is_empty() {
                                match std::fs::read(tx.trim()).map_err(|e| e.to_string()).and_then(|b| sex_3d::io::decode_image(tx.trim().rsplit('/').next().unwrap_or("Image"), &b)) {
                                    Ok(img) => {
                                        d.push_undo("Open Image");
                                        let ii = d.scene.images.len();
                                        d.scene.images.push(img);
                                        t.nodes[n].props[k].1 = Prop::Id(Some(ii));
                                        ed.change("Open Image", key);
                                        s.undo_for = Some(key);
                                    }
                                    Err(e) => anim::report(d, ReportKind::Error, &format!("Cannot open {}: {e}", tx.trim())),
                                }
                            }
                        } else {
                            s.open_path = Some((n, tx));
                        }
                        return;
                    }
                }
            }
            let br = Rect::new(w.x, w.y, (24.0 * s.zoom).max(18.0) as i32, w.h);
            if ui.dropdown(br, "") {
                let mut items: Vec<MenuItem> = names.iter().enumerate().map(|(j, nm)| MenuItem::op(nm, "", Op::with("node.set_prop", &format!("{}\t{n}\t{name}\t{j}", tref.encode())))).collect();
                if items.is_empty() {
                    items.push(MenuItem::Heading(format!("No {what}s")));
                }
                ui.open_menu(br, &format!("Browse {what}"), items);
            }
            let rest = Rect::new(br.right() + 2, w.y, w.right() - br.right() - 2, w.h);
            match v.and_then(|i| names.get(i)) {
                Some(nm) => {
                    ui.p.round_rect(rest, 4, t::BUTTON_DARK);
                    ui.p.label(Rect::new(rest.x + 6, rest.y, rest.w - 8, rest.h), nm, t::TEXT, Align::Left);
                }
                None if name == "image" => {
                    if ui.button(rest, "Open") {
                        s.open_path = Some((n, String::new()));
                    }
                }
                None => {
                    ui.p.round_rect(rest, 4, t::BUTTON_DARK);
                    ui.p.label(Rect::new(rest.x + 6, rest.y, rest.w - 8, rest.h), what, t::TEXT_DIM, Align::Left);
                }
            }
        }
        Prop::Ramp(mut ramp) => {
            let base = format!("node{}/{}/{}/ramp{k}", s.id, tref.encode(), node.name);
            let mut active = *s.ramp_active.get(&node.name).unwrap_or(&0);
            let area = Rect::new(w.x, rr.y + 1, w.w, rr.h - 2);
            let sid = s.id;
            let op = move |v: &str| anim::mail("node.ramp_interp", sid, &format!("{n}\t{k}\t{v}"));
            let (changed, ask) = widgets::ramp(ui, &base, area, row, &mut ramp, &mut active, &op);
            s.ramp_active.insert(node.name.clone(), active);
            if changed {
                t.nodes[n].props[k].1 = Prop::Ramp(ramp);
                ed.change("Color Ramp", id(&base));
            }
            if let widgets::RampAsk::Pick(e, at) = ask {
                s.picker = Some(Picker::new(ColorTarget::Node(tref, n, NodeColor::RampStop(k, e)), at, d));
                s.picker_undo = None;
            }
        }
        Prop::Curve(mut cm) => {
            let base = format!("node{}/{}/{}/curve{k}", s.id, tref.encode(), node.name);
            let mut chan = *s.curve_chan.get(&node.name).unwrap_or(&0);
            let mut sel = s.curve_sel.get(&node.name).copied();
            let area = Rect::new(w.x, rr.y + 1, w.w, rr.h - 4);
            if widgets::curve(ui, &base, area, row, &mut cm, &mut chan, &mut sel) {
                t.nodes[n].props[k].1 = Prop::Curve(cm);
                ed.change("Curve", id(&base));
            }
            s.curve_chan.insert(node.name.clone(), chan);
            match sel {
                Some(v) => s.curve_sel.insert(node.name.clone(), v),
                None => s.curve_sel.remove(&node.name),
            };
        }
    }
}

// ---------------------------------------------------------------- canvas input

/// Running modal actions see events before any widget.
#[allow(clippy::too_many_arguments)]
fn modal(s: &mut State, d: &mut Doc, ui: &mut Ui, t: &mut NodeTree, lay: &[NodeLayout], ed: &mut Ed, tref: TreeRef) {
    let Some(input) = ui.input else { return };
    let z = s.zoom;
    match (s.drag.clone(), input) {
        (Drag::Move { x, y, orig, release_confirms, key }, inp) => {
            match inp {
                Input::Move { x: mx, y: my } => {
                    let (dx, dy) = ((mx - x) as f32 / z, -(my - y) as f32 / z);
                    for (i, p) in &orig {
                        if let Some(nd) = t.nodes.get_mut(*i) {
                            nd.location = [p[0] + dx, p[1] + dy];
                        }
                    }
                    ed.change("Move", key);
                }
                Input::Down { button: Button::Left, .. } | Input::Key { key: Key::Enter | Key::NumEnter, .. } => {
                    attach_to_frames(t, &orig, lay, &s.view);
                    ed.change("Move", key);
                    s.drag = Drag::None;
                }
                Input::Up { button: Button::Left, .. } if release_confirms => {
                    attach_to_frames(t, &orig, lay, &s.view);
                    ed.change("Move", key);
                    s.drag = Drag::None;
                }
                Input::Down { button: Button::Right, .. } | Input::Key { key: Key::Esc, .. } => {
                    for (i, p) in &orig {
                        if let Some(nd) = t.nodes.get_mut(*i) {
                            nd.location = *p;
                        }
                    }
                    if s.undo_for == Some(key) {
                        ed.change("Move", key);
                    }
                    s.drag = Drag::None;
                }
                _ => return,
            }
            ui.input = None;
        }
        (Drag::Pan(lx, ly), Input::Move { x, y }) => {
            s.center.0 -= (x - lx) as f32 / z;
            s.center.1 += (y - ly) as f32 / z;
            s.drag = Drag::Pan(x, y);
            ui.input = None;
        }
        (Drag::Pan(..), Input::Up { button: Button::Middle, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Press { x: px, y: py, node, extend, .. }, Input::Move { x, y }) => {
            if (x - px).abs() + (y - py).abs() > 3 {
                if node.is_some() && !s.box_armed {
                    s.mouse = (px, py);
                    let key = s.next_key();
                    start_move(s, t, true, key);
                    if let Drag::Move { x: ox, y: oy, orig, .. } = s.drag.clone() {
                        let (dx, dy) = ((x - ox) as f32 / z, -(y - oy) as f32 / z);
                        for (i, p) in &orig {
                            t.nodes[*i].location = [p[0] + dx, p[1] + dy];
                        }
                        ed.change("Move", key);
                    }
                } else {
                    s.box_armed = false;
                    s.drag = Drag::Box { x0: px, y0: py, x1: x, y1: y, extend };
                }
            }
            s.mouse = (x, y);
            ui.input = None;
        }
        (Drag::Press { node, extend, was_selected, .. }, Input::Up { button: Button::Left, .. }) => {
            match node {
                Some(n) if was_selected && !extend => {
                    edit::select_only(t, &[n]);
                    t.active = Some(n);
                    ed.select = true;
                }
                None if !extend => {
                    t.nodes.iter_mut().for_each(|n| n.select = false);
                    ed.select = true;
                }
                _ => {}
            }
            s.box_armed = false;
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Box { x0, y0, extend, .. }, Input::Move { x, y }) => {
            s.drag = Drag::Box { x0, y0, x1: x, y1: y, extend };
            ui.input = None;
        }
        (Drag::Box { x0, y0, x1, y1, extend }, Input::Up { button: Button::Left, .. }) => {
            let b = Rect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs() + 1, (y1 - y0).abs() + 1);
            for (i, l) in lay.iter().enumerate() {
                let hit = if t.nodes[i].idname == edit::FRAME {
                    // A frame is caught only when the box holds it whole.
                    b.intersect(&l.rect) == l.rect
                } else {
                    !l.rect.intersect(&b).is_empty()
                };
                if hit {
                    t.nodes[i].select = true;
                } else if !extend {
                    t.nodes[i].select = false;
                }
            }
            ed.select = true;
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::Link { from, to, key, .. }, Input::Move { x, y }) => {
            s.drag = Drag::Link { from, to, x, y, key };
            ui.input = None;
        }
        (Drag::Link { from, to, key, .. }, Input::Up { x, y, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
            finish_link(s, d, t, lay, ed, tref, from, to, x, y, key);
        }
        (Drag::Knife(mut pts), Input::Move { x, y }) | (Drag::Reroute(mut pts), Input::Move { x, y }) => {
            pts.push((x, y));
            s.drag = if matches!(s.drag, Drag::Knife(_)) { Drag::Knife(pts) } else { Drag::Reroute(pts) };
            ui.input = None;
        }
        (Drag::Knife(pts), Input::Up { .. }) => {
            s.drag = Drag::None;
            ui.input = None;
            let crossed = crossed_links(t, lay, &pts, z);
            if !crossed.is_empty() {
                let cut: Vec<Link> = crossed.iter().map(|c| t.links[c.0]).collect();
                t.links.retain(|l| !cut.contains(l));
                let key = s.next_key();
                ed.change("Cut Links", key);
            }
        }
        (Drag::Reroute(pts), Input::Up { .. }) => {
            s.drag = Drag::None;
            ui.input = None;
            let crossed = crossed_links(t, lay, &pts, z);
            if !crossed.is_empty() {
                // One reroute per output socket crossed, where its links cross.
                let mut by_src: Vec<((usize, usize), Vec<usize>, (f32, f32))> = Vec::new();
                for (li, p) in crossed {
                    let l = t.links[li];
                    match by_src.iter_mut().find(|b| b.0 == (l.from_node, l.from_socket)) {
                        Some(b) => b.1.push(li),
                        None => by_src.push(((l.from_node, l.from_socket), vec![li], p)),
                    }
                }
                let old = t.links.clone();
                for (src, links, p) in by_src {
                    let at = s.view.to_node(p.0 as i32, p.1 as i32);
                    let Some(rn) = edit::add_node(t, edit::REROUTE, [at.0 - layout::REROUTE_R, at.1 + layout::REROUTE_R], &[], None) else { continue };
                    let ty = t.nodes[src.0].outputs[src.1].ty;
                    t.nodes[rn].inputs[0].ty = ty;
                    t.nodes[rn].outputs[0].ty = ty;
                    t.links.push(Link { from_node: src.0, from_socket: src.1, to_node: rn, to_socket: 0, muted: false });
                    for li in links {
                        let l = old[li];
                        if let Some(x) = t.links.iter_mut().find(|x| **x == l) {
                            x.from_node = rn;
                            x.from_socket = 0;
                        }
                    }
                }
                let key = s.next_key();
                ed.change("Add Reroute", key);
            }
        }
        (Drag::Resize { node, left, x: x0, orig_w, orig_x, key }, Input::Move { x, .. }) => {
            let dx = (x - x0) as f32 / z;
            if let Some(nd) = t.nodes.get_mut(node) {
                if left {
                    let w = (orig_w - dx).clamp(80.0, 700.0);
                    nd.location[0] = orig_x + (orig_w - w);
                    nd.width = w;
                } else {
                    nd.width = (orig_w + dx).clamp(80.0, 700.0);
                }
            }
            ed.change("Resize Node", key);
            ui.input = None;
        }
        (Drag::Resize { .. }, Input::Up { .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        (Drag::None, _) => {}
        (_, Input::Key { key: Key::Esc, .. }) => {
            s.drag = Drag::None;
            ui.input = None;
        }
        _ => {}
    }
}

/// Links crossed by a stroke: (link index, crossing point).
fn crossed_links(t: &NodeTree, lay: &[NodeLayout], pts: &[(i32, i32)], z: f32) -> Vec<(usize, (f32, f32))> {
    let mut out = Vec::new();
    for (li, l) in t.links.iter().enumerate() {
        let (Some(a), Some(b)) = (lay.get(l.from_node).and_then(|x| x.outputs.get(l.from_socket).copied().flatten()), lay.get(l.to_node).and_then(|x| x.inputs.get(l.to_socket).copied().flatten())) else { continue };
        let curve = layout::link_points(a, b, z, 28);
        'find: for s in pts.windows(2) {
            let (p, q) = ((s[0].0 as f32, s[0].1 as f32), (s[1].0 as f32, s[1].1 as f32));
            for c in curve.windows(2) {
                if let Some(x) = layout::seg_cross(p, q, c[0], c[1]) {
                    out.push((li, x));
                    break 'find;
                }
            }
        }
    }
    out
}

/// Blender's translate-attach: nodes dropped inside a frame join it.
fn attach_to_frames(t: &mut NodeTree, moved: &[(usize, [f32; 2])], lay: &[NodeLayout], v: &View) {
    let moved_ids: Vec<usize> = moved.iter().map(|m| m.0).collect();
    for &(i, _) in moved {
        if i >= t.nodes.len() || t.nodes[i].parent.is_some() || t.nodes[i].idname == edit::FRAME {
            continue;
        }
        let w = t.nodes[i].width;
        let h = lay.get(i).map_or(40.0, |l| l.height);
        let c = (t.nodes[i].location[0] + w / 2.0, t.nodes[i].location[1] - h / 2.0);
        let (cx, cy) = v.to_screen(c);
        let frame = (0..t.nodes.len()).rev().find(|&f| t.nodes[f].idname == edit::FRAME && !moved_ids.contains(&f) && lay.get(f).is_some_and(|l| l.rect.contains(cx as i32, cy as i32)));
        if let Some(f) = frame {
            t.nodes[i].parent = Some(f);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_link(s: &mut State, d: &mut Doc, t: &mut NodeTree, lay: &[NodeLayout], ed: &mut Ed, tref: TreeRef, from: Option<(usize, usize)>, to: Option<(usize, usize)>, x: i32, y: i32, key: u64) {
    let order = draw_order(t);
    let target = layout::socket_at(lay, x, y, s.zoom).or_else(|| {
        // Dropped on a node: its first free socket of a fitting type.
        let n = node_at(t, lay, &order, x, y)?;
        match (from, to) {
            (Some((a, o)), _) if n != a => {
                let ty = t.nodes[a].outputs[o].ty;
                let cands: Vec<usize> = (0..t.nodes[n].inputs.len()).filter(|&i| lay[n].inputs[i].is_some() && t.nodes[n].inputs[i].ty != SocketType::Virtual).collect();
                cands
                    .iter()
                    .copied()
                    .find(|&i| t.nodes[n].inputs[i].ty == ty && !layout::is_linked(t, n, i))
                    .or_else(|| cands.iter().copied().find(|&i| edit::can_link(ty, t.nodes[n].inputs[i].ty) && !layout::is_linked(t, n, i)))
                    .map(|i| (n, i, true))
            }
            (None, Some((b, i))) if n != b => {
                let ty = t.nodes[b].inputs[i].ty;
                (0..t.nodes[n].outputs.len()).find(|&o| lay[n].outputs[o].is_some() && edit::can_link(t.nodes[n].outputs[o].ty, ty)).map(|o| (n, o, false))
            }
            _ => None,
        }
    });
    let (a, o, b, i) = match (from, to, target) {
        (Some((a, o)), _, Some((b, i, true))) => (a, o, b, i),
        (None, Some((b, i)), Some((a, o, false))) => (a, o, b, i),
        _ => return,
    };
    if a == b {
        return;
    }
    let fty = t.nodes[a].outputs[o].ty;
    let tty = t.nodes[b].inputs[i].ty;
    // Linking to an empty socket of Group Input/Output adds an interface socket.
    if fty == SocketType::Virtual || tty == SocketType::Virtual {
        let Some(g) = tref.group() else { return };
        let (input_side, ty, name) = if fty == SocketType::Virtual {
            (true, tty, t.nodes[b].inputs[i].name.clone())
        } else {
            (false, fty, t.nodes[a].outputs[o].name.clone())
        };
        if ty == SocketType::Virtual {
            return;
        }
        // Push what changed so far, then edit the group in the document.
        commit(s, d, tref, t.clone(), ed);
        if s.undo_for != Some(key) {
            d.push_undo("Add Group Socket");
            s.undo_for = Some(key);
        }
        let gr = &mut d.scene.node_groups[g];
        let len = if input_side { gr.inputs.len() } else { gr.outputs.len() };
        let sock = sex_3d::nodes::InterfaceSocket { name, ty, default: edit::type_default(ty), min: f32::MIN, max: f32::MAX };
        if input_side {
            gr.inputs.push(sock);
        } else {
            gr.outputs.push(sock);
        }
        nodetree::interface_changed(&mut d.scene, g, input_side, &|k| Some(if k >= len { k + 1 } else { k }));
        if let Some(tt) = tref.get_mut(&mut d.scene) {
            let (o2, i2) = if input_side { (len, i) } else { (o, len) };
            let _ = tt.link_sockets(a, o2, b, i2);
        }
        nodetree::after_edit(d, tref);
        ed.reload = true;
        return;
    }
    if !edit::can_link(fty, tty) {
        anim::report(d, ReportKind::Warning, &format!("Cannot link {fty:?} to {tty:?}"));
        return;
    }
    match t.link_sockets(a, o, b, i) {
        Ok(_) => ed.change("Link Nodes", key),
        Err(e) => anim::report(d, ReportKind::Warning, &e.to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn canvas_input(s: &mut State, d: &mut Doc, ui: &mut Ui, t: &mut NodeTree, lay: &[NodeLayout], order: &[usize], ed: &mut Ed, r: Rect) {
    let Some(input) = ui.input else { return };
    if !matches!(s.drag, Drag::None) {
        return;
    }
    let clip = ui.p.clip();
    let inside = |x: i32, y: i32| r.contains(x, y) && clip.contains(x, y);
    let z = s.zoom;
    match input {
        Input::Down { x, y, button, mods } if inside(x, y) => {
            let knife = (button == Button::Right && mods.ctrl && !mods.shift)
                || (button == Button::Middle && mods.ctrl && !mods.shift)
                || (button == Button::Left && mods.ctrl && mods.alt && !mods.shift);
            let reroute = (button == Button::Right && mods.shift && !mods.ctrl)
                || (button == Button::Middle && mods.ctrl && mods.shift)
                || (button == Button::Left && mods.shift && mods.alt && mods.ctrl);
            if knife {
                s.drag = Drag::Knife(vec![(x, y)]);
            } else if reroute {
                s.drag = Drag::Reroute(vec![(x, y)]);
            } else if button == Button::Middle {
                s.drag = Drag::Pan(x, y);
            } else if button == Button::Right {
                // Blender's node context menu.
                let (_, _, _, items) = menus(s, d);
                let mut p = crate::ui::Popup::new("Node", items, x, y);
                p.owner = Rect::default();
                d.menu = Some(p);
            } else if let Some((n, i, is_input)) = layout::socket_at(lay, x, y, z) {
                let key = s.next_key();
                if is_input {
                    match t.input_link(n, i).copied().or_else(|| t.links.iter().find(|l| l.to_node == n && l.to_socket == i).copied()) {
                        Some(l) => {
                            // Pick the link up from its input.
                            t.unlink_input(n, i);
                            ed.change("Link Nodes", key);
                            s.drag = Drag::Link { from: Some((l.from_node, l.from_socket)), to: None, x, y, key };
                        }
                        None => s.drag = Drag::Link { from: None, to: Some((n, i)), x, y, key },
                    }
                } else {
                    s.drag = Drag::Link { from: Some((n, i)), to: None, x, y, key };
                }
            } else if let Some(n) = node_at(t, lay, order, x, y) {
                let rr = lay[n].rect;
                let edge = t.nodes[n].idname != edit::FRAME && t.nodes[n].idname != edit::REROUTE && !s.box_armed;
                let hh = (HEAD * z) as i32;
                if edge && y > rr.y + hh / 2 && ((x - rr.x).abs() <= 4 || (x - rr.right()).abs() <= 4) {
                    let key = s.next_key();
                    s.drag = Drag::Resize { node: n, left: (x - rr.x).abs() <= 4, x, orig_w: t.nodes[n].width, orig_x: t.nodes[n].location[0], key };
                } else {
                    let was = t.nodes[n].select;
                    if mods.shift {
                        if was && t.active == Some(n) {
                            t.nodes[n].select = false;
                        } else {
                            t.nodes[n].select = true;
                            t.active = Some(n);
                        }
                    } else {
                        if !was {
                            edit::select_only(t, &[n]);
                        }
                        t.active = Some(n);
                    }
                    ed.select = true;
                    s.drag = Drag::Press { x, y, node: if s.box_armed { None } else { Some(n) }, extend: mods.shift, was_selected: was };
                }
            } else {
                s.drag = Drag::Press { x, y, node: None, extend: mods.shift, was_selected: false };
            }
            ui.input = None;
        }
        Input::Wheel { x, y, steps, mods } if inside(x, y) => {
            if mods.ctrl {
                s.center.0 -= steps as f32 * 40.0 / z;
            } else if mods.shift {
                s.center.1 += steps as f32 * 40.0 / z;
            } else {
                let before = s.view.to_node(x, y);
                s.zoom = (s.zoom * 1.15f32.powi(steps)).clamp(0.15, 3.0);
                let v = View { center: s.center, zoom: s.zoom, region: r };
                let after = v.to_node(x, y);
                s.center = (s.center.0 + before.0 - after.0, s.center.1 + before.1 - after.1);
            }
            ui.input = None;
        }
        _ => {}
    }
}

/// Keys while the pointer is over the editor (Blender's node editor keymap).
pub fn hotkey(s: &mut State, d: &mut Doc, i: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *i else { return false };
    let alt_ctrl = Mods { shift: false, ctrl: true, alt: true };
    let op = match (key, mods) {
        (Key::Home, Mods::NONE) => "node.view_all",
        (Key::NumDot, Mods::NONE) => "node.view_selected",
        (Key::Char('a'), Mods::NONE) => "node.select_all",
        (Key::Char('a'), Mods::ALT) => "node.select_none",
        (Key::Char('i'), Mods::CTRL) => "node.select_invert",
        (Key::Char('b'), Mods::NONE) => "node.select_box",
        (Key::Char('l'), Mods::NONE) => "node.select_linked_to",
        (Key::Char('l'), Mods::SHIFT) => "node.select_linked_from",
        (Key::Char('f'), Mods::CTRL) => "node.find_node",
        (Key::Char('g'), Mods::NONE) => "node.translate",
        (Key::Char('x'), Mods::NONE) | (Key::Delete, Mods::NONE) => "node.delete",
        (Key::Char('x'), Mods::CTRL) => "node.delete_reconnect",
        (Key::Char('d'), Mods::SHIFT) => "node.duplicate_move",
        (Key::Char('d'), Mods::CTRL_SHIFT) => "node.duplicate_move_keep_inputs",
        (Key::Char('c'), Mods::CTRL) => "node.clipboard_copy",
        (Key::Char('v'), Mods::CTRL) => "node.clipboard_paste",
        (Key::Char('a'), Mods::SHIFT) => "node.add_menu",
        (Key::Char('m'), Mods::NONE) => "node.mute_toggle",
        (Key::Char('h'), Mods::NONE) => "node.hide_toggle",
        (Key::Char('h'), Mods::CTRL) => "node.hide_socket_toggle",
        (Key::Char('f'), Mods::NONE) => "node.link_make",
        (Key::Char('f'), Mods::SHIFT) => {
            d.queue.push(anim::mail("node.link_make", s.id, "REPLACE"));
            return true;
        }
        (Key::Char('j'), Mods::CTRL) => "node.join",
        (Key::Char('p'), Mods::ALT) => "node.detach",
        (Key::Char('g'), Mods::CTRL) => "node.group_make",
        (Key::Char('g'), m) if m == alt_ctrl => "node.group_ungroup",
        (Key::Tab, Mods::NONE) => "node.group_edit",
        (Key::Char('n'), Mods::NONE) => "node.sidebar",
        _ => return false,
    };
    d.queue.push(anim::mail(op, s.id, ""));
    true
}
