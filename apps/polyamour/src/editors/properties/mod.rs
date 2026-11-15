// SPDX-License-Identifier: GPL-3.0-only
//! The Properties editor, laid out like Blender's: the vertical tab column on
//! the left (Tool, Render, Output, View Layer, Scene, World, Collection,
//! Object, Modifiers, Particles, Physics, Object Constraints, Object Data,
//! Material, Texture; shown by the active object's type), a header with the
//! property search field and the pin, and collapsible panels and sub-panels.
//!
//! Every edit is one undo step and bumps the document generation so the
//! viewports redraw. Dropdown menus queue `properties.set key=VALUE` (and a
//! few Blender operators such as `object.modifier_remove`) which this editor
//! applies at the start of its next draw, before anything else runs them.
mod data;
pub mod icons;
pub mod layout;
pub mod nodepanel;
mod object;
pub mod picker;
mod tabs;

pub use object::{rename_object, set_parent};

use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::{Input, Key, Mods};
use crate::theme as t;
use crate::ui::{id, Op, Ui};
use icons::Icon;
use layout::L;
use sex_3d::scene::ObjectData;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tab {
    Tool,
    Render,
    Output,
    ViewLayer,
    Scene,
    World,
    Collection,
    Object,
    Modifiers,
    Particles,
    Physics,
    Constraints,
    Data,
    Material,
    Texture,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Tool => "Tool",
            Tab::Render => "Render",
            Tab::Output => "Output",
            Tab::ViewLayer => "View Layer",
            Tab::Scene => "Scene",
            Tab::World => "World",
            Tab::Collection => "Collection",
            Tab::Object => "Object",
            Tab::Modifiers => "Modifiers",
            Tab::Particles => "Particles",
            Tab::Physics => "Physics",
            Tab::Constraints => "Object Constraints",
            Tab::Data => "Data",
            Tab::Material => "Material",
            Tab::Texture => "Texture",
        }
    }
    fn icon(self, data: Icon) -> Icon {
        match self {
            Tab::Tool => Icon::Tool,
            Tab::Render => Icon::Render,
            Tab::Output => Icon::Output,
            Tab::ViewLayer => Icon::ViewLayer,
            Tab::Scene => Icon::Scene,
            Tab::World => Icon::World,
            Tab::Collection => Icon::Collection,
            Tab::Object => Icon::Object,
            Tab::Modifiers => Icon::Modifier,
            Tab::Particles => Icon::Particles,
            Tab::Physics => Icon::Physics,
            Tab::Constraints => Icon::Constraint,
            Tab::Data => data,
            Tab::Material => Icon::Material,
            Tab::Texture => Icon::Texture,
        }
    }
}

/// The tabs Blender shows for the current context, with group breaks (None).
pub fn tabs_for(d: &Doc, obj: Option<usize>) -> Vec<Option<Tab>> {
    let mut v = vec![Some(Tab::Tool), None, Some(Tab::Render), Some(Tab::Output), Some(Tab::ViewLayer), Some(Tab::Scene), Some(Tab::World), Some(Tab::Collection), None];
    if let Some(o) = obj {
        v.push(Some(Tab::Object));
        let data = &d.scene.objects[o].data;
        let mesh = matches!(data, ObjectData::Mesh(_));
        if mesh {
            v.extend([Some(Tab::Modifiers), Some(Tab::Particles)]);
        }
        if mesh || matches!(data, ObjectData::Empty) {
            v.push(Some(Tab::Physics));
        }
        v.extend([Some(Tab::Constraints), Some(Tab::Data)]);
        if mesh {
            v.push(Some(Tab::Material));
        }
    }
    v.push(Some(Tab::Texture));
    v
}

/// Something a colour picker edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorTarget {
    BaseColor(usize),
    Emission(usize),
    World,
    Light(usize),
    /// A colour inside a node of a node tree.
    Node(crate::nodetree::TreeRef, usize, crate::nodetree::NodeColor),
}

/// Per-frame context the panels share.
pub struct Ctx {
    pub obj: Option<usize>,
    pub mat_slot: usize,
    /// Camera lens shown as field of view instead of millimetres.
    pub fov_unit: bool,
    pub open_picker: Option<(ColorTarget, Rect)>,
}

pub struct State {
    pub tab: Tab,
    scroll: HashMap<Tab, i32>,
    content_h: i32,
    pub open: HashMap<String, bool>,
    pub search: String,
    focus_search: bool,
    /// Pinned object name: object tabs keep showing it.
    pub pin: Option<String>,
    pub mat_slot: usize,
    pub fov_unit: bool,
    pub picker: Option<picker::Picker>,
    undo_for: Option<Option<u64>>,
    /// Last frame's property buttons by key/label, and the tab buttons.
    pub fields: Vec<(String, Rect)>,
    pub tab_rects: Vec<(Tab, Rect)>,
}

impl State {
    /// Where the property `key` (or label) was drawn last frame.
    pub fn field(&self, key: &str) -> Option<Rect> {
        self.fields.iter().find(|f| f.0 == key).map(|f| f.1)
    }
}

impl Default for State {
    fn default() -> Self {
        // Blender's factory startup opens on the default cube's Object tab.
        State {
            tab: Tab::Object,
            scroll: HashMap::new(),
            content_h: 0,
            open: HashMap::new(),
            search: String::new(),
            focus_search: false,
            pin: None,
            mat_slot: 0,
            fov_unit: false,
            picker: None,
            undo_for: None,
            fields: Vec::new(),
            tab_rects: Vec::new(),
        }
    }
}

const TAB_W: i32 = 32;
const SEARCH_ID: &str = "props/search";

fn ctx_object(s: &State, d: &Doc) -> Option<usize> {
    match &s.pin {
        Some(name) => d.scene.find_object(name),
        None => d.active(),
    }
}

pub fn header(s: &mut State, _d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let pin_r = Rect::new(hr.right() - 30, hr.y + 3, 24, hr.h - 6);
    let sr = Rect::new(hr.x + 4, hr.y + 3, (pin_r.x - 8 - hr.x - 4).max(40), hr.h - 6);
    let wid = id(SEARCH_ID);
    if s.focus_search {
        s.focus_search = false;
        ui.st.editing = Some((wid, s.search.clone()));
    }
    let editing = matches!(&ui.st.editing, Some((e, _)) if *e == wid);
    let mut text = s.search.clone();
    let inner = Rect::new(sr.x + 18, sr.y, sr.w - 18, sr.h);
    ui.p.round_rect(sr, 4, t::BUTTON_DARK);
    ui.text_field(wid, inner, &mut text);
    s.search = text;
    if let Some((e, live)) = &ui.st.editing {
        if *e == wid {
            // Blender filters while you type.
            s.search = live.clone();
        }
    }
    icons::draw(ui.p, Icon::Search, sr.x + 10, sr.y + sr.h / 2);
    if !editing && s.search.is_empty() {
        ui.p.label(Rect::new(inner.x + 6, inner.y, inner.w, inner.h), "Search (Ctrl+F)", t::TEXT_DIM, Align::Left);
    }
    let on = s.pin.is_some();
    let hov = ui.hovered(pin_r);
    if on || hov {
        ui.p.round_rect(pin_r, 4, if on { t::SELECTED } else { t::BUTTON_HOVER });
    }
    icons::draw(ui.p, Icon::Pin, pin_r.x + pin_r.w / 2, pin_r.y + pin_r.h / 2);
    if ui.clicked(pin_r) {
        s.pin = if on { None } else { _d.active().map(|a| _d.scene.objects[a].name.clone()) };
    }
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if r.is_empty() {
        return;
    }
    d.settings.validate_parked(&d.scene);
    run_queued(s, d);
    let obj = ctx_object(s, d);
    let tabs = tabs_for(d, obj);
    if !tabs.contains(&Some(s.tab)) {
        // Blender falls back to the nearest tab that exists.
        s.tab = if obj.is_some() { Tab::Object } else { Tab::Scene };
    }

    // The colour picker sits on top: it sees events first.
    let mut stash = None;
    if let Some(pk) = &s.picker {
        match picker::claims(pk, ui) {
            picker::Claim::Mine => stash = ui.input.take(),
            picker::Claim::Close => s.picker = None,
            picker::Claim::No => {}
        }
    }

    // Tab column.
    let (tr, cr) = r.take_left(TAB_W);
    ui.p.fill(tr, 0x232323);
    let data_icon = match obj.map(|o| &d.scene.objects[o].data) {
        Some(ObjectData::Mesh(_)) => Icon::Mesh,
        Some(ObjectData::Camera(_)) => Icon::CameraData,
        Some(ObjectData::Light(_)) => Icon::LightData,
        _ => Icon::EmptyData,
    };
    let mut y = tr.y + 6;
    s.tab_rects.clear();
    for tab in &tabs {
        match tab {
            None => y += 8,
            Some(tab) => {
                let b = Rect::new(tr.x + 3, y, TAB_W - 6, 26);
                s.tab_rects.push((*tab, b));
                let on = *tab == s.tab;
                let hov = ui.hovered(b);
                if on || hov {
                    ui.p.round_rect(b, 4, if on { 0x4a4a4a } else { 0x333333 });
                }
                icons::draw(ui.p, tab.icon(data_icon), b.x + b.w / 2, b.y + b.h / 2);
                if hov {
                    ui.st.hint = tab.label().to_string();
                }
                if ui.clicked(b) {
                    s.tab = *tab;
                    s.picker = None;
                }
                y += 28;
            }
        }
    }

    // Content.
    ui.p.fill(cr, t::PROPERTIES);
    ui.p.push_clip(cr);
    let tab = s.tab;
    let scroll = s.scroll.get(&tab).copied().unwrap_or(0);
    let mut ctx = Ctx { obj, mat_slot: s.mat_slot, fov_unit: s.fov_unit, open_picker: None };
    let query = s.search.trim().to_lowercase();
    let mut l = L {
        ui: &mut *ui,
        open: &mut s.open,
        x: cr.x + 6,
        w: cr.w - 14,
        y: cr.y + 6 - scroll,
        query,
        dry: false,
        hits: 0,
        depth: 0,
        inactive: false,
        prefix: format!("props/{}", tab.label()),
        undo_for: s.undo_for,
        fields: Vec::new(),
    };
    let top = l.y;
    context_path(&mut l, d, tab, obj);
    match tab {
        Tab::Tool => tabs::tool(&mut l, d),
        Tab::Render => tabs::render(&mut l, d),
        Tab::Output => tabs::output(&mut l, d),
        Tab::ViewLayer => tabs::view_layer(&mut l, d),
        Tab::Scene => tabs::scene(&mut l, d),
        Tab::World => tabs::world(&mut l, d, &mut ctx),
        Tab::Collection => tabs::collection(&mut l, d),
        Tab::Object => object::object(&mut l, d, &mut ctx),
        Tab::Modifiers => object::modifiers(&mut l, d, &mut ctx),
        Tab::Particles => tabs::particles(&mut l, d),
        Tab::Physics => tabs::physics(&mut l, d),
        Tab::Constraints => tabs::constraints(&mut l, d),
        Tab::Data => data::data(&mut l, d, &mut ctx),
        Tab::Material => data::material(&mut l, d, &mut ctx),
        Tab::Texture => tabs::texture(&mut l, d),
    }
    // Panels scripts and add-ons registered for this tab (bl_context).
    let context = match tab {
        Tab::Tool => "tool",
        Tab::Render => "render",
        Tab::Output => "output",
        Tab::ViewLayer => "view_layer",
        Tab::Scene => "scene",
        Tab::World => "world",
        Tab::Collection => "collection",
        Tab::Object => "object",
        Tab::Modifiers => "modifier",
        Tab::Particles => "particle",
        Tab::Physics => "physics",
        Tab::Constraints => "constraint",
        Tab::Data => "data",
        Tab::Material => "material",
        Tab::Texture => "texture",
    };
    crate::python::properties_panels(&mut l, d, context);
    let content_h = l.y - top + 12;
    s.undo_for = if l.ui.st.active.is_none() { None } else { l.undo_for };
    s.fields = core::mem::take(&mut l.fields);
    s.content_h = content_h;
    s.mat_slot = ctx.mat_slot;
    s.fov_unit = ctx.fov_unit;
    if let Some((target, anchor)) = ctx.open_picker {
        s.picker = Some(picker::Picker::new(target, anchor, d));
    }
    // Wheel scrolls the panels.
    let max = (content_h - cr.h).max(0);
    let mut sc = scroll;
    if let Some(Input::Wheel { x, y, steps, .. }) = ui.input {
        if cr.contains(x, y) {
            ui.input = None;
            sc -= steps * 40;
        }
    }
    s.scroll.insert(tab, sc.clamp(0, max));
    // Scrollbar like Blender's thin one.
    if max > 0 {
        let vis = cr.h as f32 / content_h as f32;
        let bh = ((cr.h as f32 * vis) as i32).max(20);
        let by = cr.y + ((cr.h - bh) as f32 * (sc.clamp(0, max) as f32 / max as f32)) as i32;
        ui.p.round_rect(Rect::new(cr.right() - 5, by, 3, bh), 1, 0x5a5a5a);
    }
    ui.p.pop_clip();

    if stash.is_some() {
        ui.input = stash;
    }
    if let Some(pk) = &mut s.picker {
        if !picker::draw(pk, d, ui, r, &mut s.undo_for) {
            s.picker = None;
        }
    }
}

/// The breadcrumb Blender shows at the top of object tabs ("Cube > Cube").
fn context_path(l: &mut L, d: &Doc, tab: Tab, obj: Option<usize>) {
    let Some(o) = obj else { return };
    if !matches!(tab, Tab::Object | Tab::Modifiers | Tab::Particles | Tab::Physics | Tab::Constraints | Tab::Data | Tab::Material) || l.dry {
        return;
    }
    let ob = &d.scene.objects[o];
    let x = l.x + 4;
    let y = l.y;
    l.ui.p.fill(Rect::new(l.x, y, l.w, 22), t::PROPERTIES);
    let oi = object_icon(&ob.data);
    icons::draw(l.ui.p, oi, x + 8, y + 11);
    let mut tx = l.ui.p.text(x + 18, y + 4, &ob.name, t::TEXT);
    let sub: Option<(Icon, String)> = match (tab, &ob.data) {
        (Tab::Modifiers, _) => Some((Icon::Modifier, "Modifiers".into())),
        (Tab::Data, ObjectData::Mesh(mi)) => Some((Icon::Mesh, d.scene.mesh_names.get(*mi).cloned().unwrap_or_default())),
        (Tab::Data, ObjectData::Camera(_)) => Some((Icon::CameraData, ob.name.clone())),
        (Tab::Data, ObjectData::Light(_)) => Some((Icon::LightData, ob.name.clone())),
        (Tab::Material, _) => ob.materials.first().and_then(|&m| d.scene.materials.get(m)).map(|m| (Icon::Material, m.name.clone())),
        _ => None,
    };
    if let Some((ic, name)) = sub {
        l.ui.p.arrow((tx + 10) as f32, (y + 11) as f32, 7.0, false, t::TEXT_DIM);
        tx += 20;
        icons::draw(l.ui.p, ic, tx + 8, y + 11);
        l.ui.p.text(tx + 18, y + 4, &name, t::TEXT);
    }
    l.y += 26;
}

pub fn object_icon(data: &ObjectData) -> Icon {
    match data {
        ObjectData::Mesh(_) => Icon::ObMesh,
        ObjectData::Camera(_) => Icon::ObCamera,
        ObjectData::Light(_) => Icon::ObLight,
        ObjectData::Empty | ObjectData::Armature(_) | ObjectData::Lattice(_) => Icon::ObEmpty,
    }
}

/// Apply operators the editor's menus queued (see the module docs).
fn run_queued(s: &mut State, d: &mut Doc) {
    // Modifier operators from our cards carry the card's index; others
    // (e.g. a viewport menu naming the modifier) are left to the document.
    let mine = |op: &Op| {
        op.id.starts_with("properties.")
            || (matches!(op.id.as_str(), "object.modifier_apply" | "object.modifier_copy" | "object.modifier_remove" | "object.modifier_move_up" | "object.modifier_move_down" | "object.modifier_move_to_first" | "object.modifier_move_to_last")
                && op.arg.parse::<usize>().is_ok())
    };
    if !d.queue.iter().any(mine) {
        return;
    }
    let (ops, rest): (Vec<Op>, Vec<Op>) = core::mem::take(&mut d.queue).into_iter().partition(mine);
    d.queue = rest;
    let obj = ctx_object(s, d);
    for op in ops {
        match op.id.as_str() {
            "properties.set" => {
                if let Some((k, v)) = op.arg.split_once('=') {
                    set_enum(s, d, obj, k, v);
                }
            }
            "properties.unavailable" => d.report = format!("{} is not available yet", op.arg),
            id if id.starts_with("object.modifier_") => {
                if let Some(o) = obj {
                    object::modifier_op(d, o, id, op.arg.parse().unwrap_or(0));
                }
            }
            _ => d.report = format!("{} is not available yet", op.id),
        }
    }
    // Ask the window to draw again so every editor shows the change.
    d.queue.push(Op::with("screen.frame_offset", "0"));
}

/// Enum properties picked from dropdowns.
fn set_enum(s: &mut State, d: &mut Doc, obj: Option<usize>, key: &str, v: &str) {
    use crate::settings::{Engine, RotationMode, UnitSystem};
    use sex_3d::render::{MistFalloff, ShutterPosition, ViewTransform};
    use sex_3d::scene::{AreaShape, PanoramaType, SensorFit};
    let st = &mut d.settings;
    match key {
        "engine" => st.engine = if v == Engine::Workbench.label() { Engine::Workbench } else { Engine::PathTracer },
        "view_transform" => {
            let Some(t) = ViewTransform::from_label(v) else { return d.report = format!("{v} view transform is not available") };
            d.push_undo("View Transform");
            d.settings.view_transform = t;
        }
        "look" => {
            d.push_undo("Look");
            d.settings.look = v.into();
        }
        "shutter_position" => {
            d.push_undo("Shutter Position");
            d.settings.shutter_position = match v {
                "Start on Frame" => ShutterPosition::Start,
                "End on Frame" => ShutterPosition::End,
                _ => ShutterPosition::Center,
            };
        }
        "mist_falloff" => {
            d.push_undo("Mist Falloff");
            d.settings.mist_falloff = match v {
                "Linear" => MistFalloff::Linear,
                "Inverse Quadratic" => MistFalloff::InverseQuadratic,
                _ => MistFalloff::Quadratic,
            };
        }
        "units" => st.unit_system = UnitSystem::ALL.into_iter().find(|u| u.label() == v).unwrap_or(UnitSystem::Metric),
        "rotation_units" => st.degrees = v == "Degrees",
        "file_format" => st.file_format = v.into(),
        "fps" => {
            if let Some(f) = v.split(' ').next().and_then(|n| n.parse::<f32>().ok()) {
                d.push_undo("Frame Rate");
                d.scene.fps = f;
            }
        }
        "camera" => {
            d.push_undo("Scene Camera");
            d.scene.active_camera = d.scene.find_object(v);
        }
        "rotation_mode" => {
            if let Some(o) = obj {
                let name = d.scene.objects[o].name.clone();
                let mode = match v {
                    "Quaternion (WXYZ)" => RotationMode::Quaternion,
                    "Axis Angle" => RotationMode::AxisAngle,
                    "XYZ Euler" => RotationMode::EulerXyz,
                    _ => return d.report = format!("{v} rotation is not available yet; XYZ Euler, Quaternion and Axis Angle are"),
                };
                d.settings.rotation_mode.insert(name, mode);
            }
        }
        "parent" => {
            if let Some(o) = obj {
                object::set_parent(d, o, d.scene.find_object(v));
            }
        }
        "add_collection" => {
            if let (Some(o), Some(ci)) = (obj, d.scene.collections.iter().position(|c| c.name == v)) {
                d.push_undo("Link to Collection");
                d.scene.objects[o].collection = ci;
            }
        }
        "material" => {
            if let (Some(o), Some(mi)) = (obj, d.scene.materials.iter().position(|m| m.name == v)) {
                d.push_undo("Assign Material");
                let slots = &mut d.scene.objects[o].materials;
                if slots.is_empty() {
                    slots.push(mi);
                    s.mat_slot = 0;
                } else {
                    let i = s.mat_slot.min(slots.len() - 1);
                    slots[i] = mi;
                }
            }
        }
        "lens_type" => {
            if let Some(o) = obj {
                let aspect = d.settings.resolution_y as f32 / d.settings.resolution_x.max(1) as f32;
                if let ObjectData::Camera(c) = &d.scene.objects[o].data {
                    let mut nc = *c;
                    match v {
                        "Orthographic" => {
                            nc.panorama = None;
                            if nc.ortho_scale.is_none() {
                                nc.ortho_scale = Some(6.0 * aspect.min(1.0));
                            }
                        }
                        "Panoramic" => {
                            nc.ortho_scale = None;
                            nc.panorama = Some(nc.panorama.unwrap_or_default());
                        }
                        _ => {
                            nc.ortho_scale = None;
                            nc.panorama = None;
                        }
                    }
                    if nc != *c {
                        d.push_undo("Lens Type");
                        d.scene.objects[o].data = ObjectData::Camera(nc);
                    }
                }
            }
        }
        "panorama_type" | "sensor_fit" | "dof_focus_object" => {
            if let Some(o) = obj {
                let focus = d.scene.find_object(v);
                if let ObjectData::Camera(c) = &d.scene.objects[o].data {
                    let mut nc = *c;
                    match key {
                        "panorama_type" => {
                            nc.panorama = Some(match v {
                                "Fisheye Equidistant" => PanoramaType::FisheyeEquidistant,
                                "Fisheye Equisolid" => PanoramaType::FisheyeEquisolid,
                                _ => PanoramaType::Equirectangular,
                            })
                        }
                        "sensor_fit" => {
                            nc.sensor_fit = match v {
                                "Horizontal" => SensorFit::Horizontal,
                                "Vertical" => SensorFit::Vertical,
                                _ => SensorFit::Auto,
                            }
                        }
                        _ => nc.dof.focus_object = focus.filter(|f| *f != o),
                    }
                    if nc != *c {
                        d.push_undo("Camera");
                        d.scene.objects[o].data = ObjectData::Camera(nc);
                    }
                }
            }
        }
        "area_shape" => {
            if let Some(o) = obj {
                if let ObjectData::Light(l) = &d.scene.objects[o].data {
                    let mut nl = *l;
                    nl.shape = match v {
                        "Rectangle" => AreaShape::Rectangle,
                        "Disk" => AreaShape::Disk,
                        "Ellipse" => AreaShape::Ellipse,
                        _ => AreaShape::Square,
                    };
                    if nl != *l {
                        d.push_undo("Light Shape");
                        d.scene.objects[o].data = ObjectData::Light(nl);
                    }
                }
            }
        }
        "lens_unit" => s.fov_unit = v == "Field of View",
        k if k.starts_with("modobj/") => {
            // A modifier's object field (Boolean operand, Shrinkwrap target).
            if let (Some(o), Ok(si)) = (obj, k["modobj/".len()..].parse::<usize>()) {
                object::set_modifier_object(d, o, si, if v == object::NO_OBJECT { "" } else { v });
            }
        }
        _ => d.report = format!("{key} cannot be set yet"),
    }
    d.dirty = true;
    d.touch();
}

pub fn hotkey(s: &mut State, _d: &mut Doc, input: &Input, _r: Rect) -> bool {
    match *input {
        Input::Key { key: Key::Char('f'), mods: Mods::CTRL, .. } => {
            s.focus_search = true;
            true
        }
        Input::Key { key: Key::Esc, .. } if s.picker.is_some() => {
            s.picker = None;
            true
        }
        Input::Key { key: Key::Esc, .. } if !s.search.is_empty() => {
            s.search.clear();
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
