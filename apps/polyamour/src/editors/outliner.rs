// SPDX-License-Identifier: GPL-3.0-only
//! The Outliner, as Blender 4.x shows it in View Layer mode: Scene Collection,
//! its collections and their objects, each object expandable to its data
//! (with materials), modifiers and child objects. Disclosure triangles, type
//! icons, alternating rows, the selection highlight in sync with the 3D
//! Viewport (active row brighter), restriction toggles at the right (exclude
//! checkbox, eye, and the render camera when enabled in the filter popover),
//! inline rename (double-click or F2), drag and drop to move objects between
//! collections or parent them, the right-click context menus and the keymap
//! (A, Alt+A, X/Delete, H, Alt+H, F2, Numpad . / period, Home, +/-).
//!
//! The header has the display-mode dropdown (View Layer, Scenes, Blender
//! File and Orphan Data work), the search field, the filter popover and the
//! New Collection button.
//!
//! Alt+A deselects (A pressed while everything is selected does too) and
//! Alt+H unhides (Shift+H also works). A middle press with Ctrl held counts as
//! Ctrl+click here.
use super::properties::icons::{self, Icon};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::input::{Button, Input, Key, Mods};
use crate::theme as t;
use crate::ui::{id, MenuItem, Op, Ui};
use sex_3d::scene::ObjectData;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

const ROW_H: i32 = 20;
const INDENT: i32 = 18;
const COL_W: i32 = 22;
const DOUBLE_CLICK: Duration = Duration::from_millis(450);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Scenes,
    ViewLayer,
    VideoSequencer,
    BlenderFile,
    DataApi,
    LibraryOverrides,
    OrphanData,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 7] =
        [DisplayMode::Scenes, DisplayMode::ViewLayer, DisplayMode::VideoSequencer, DisplayMode::BlenderFile, DisplayMode::DataApi, DisplayMode::LibraryOverrides, DisplayMode::OrphanData];
    pub fn label(self) -> &'static str {
        match self {
            DisplayMode::Scenes => "Scenes",
            DisplayMode::ViewLayer => "View Layer",
            DisplayMode::VideoSequencer => "Video Sequencer",
            DisplayMode::BlenderFile => "Blender File",
            DisplayMode::DataApi => "Data API",
            DisplayMode::LibraryOverrides => "Library Overrides",
            DisplayMode::OrphanData => "Orphan Data",
        }
    }
    fn icon(self) -> Icon {
        match self {
            DisplayMode::Scenes => Icon::Scene,
            DisplayMode::ViewLayer => Icon::ViewLayer,
            DisplayMode::BlenderFile => Icon::Output,
            DisplayMode::OrphanData => Icon::Cross,
            _ => Icon::Dots,
        }
    }
}

/// What a row shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RowKey {
    Scene,
    SceneCol,
    Col(usize),
    Obj(usize),
    /// The object's data (mesh, camera, light).
    Data(usize),
    Mods(usize),
    Mod(usize, usize),
    /// Material slot `1` of object `0`.
    Mat(usize, usize),
    World,
    /// A Blender File category.
    Cat(u8),
    MeshDb(usize),
    MatDb(usize),
}

#[derive(Clone, Debug)]
pub struct Row {
    pub key: RowKey,
    pub depth: i32,
    pub label: String,
    pub icon: Icon,
    pub has_kids: bool,
    pub open: bool,
    open_key: String,
    /// The object the row belongs to (clicking selects it).
    pub owner: Option<usize>,
    /// Greyed: hidden or excluded.
    pub dim: bool,
    /// Icons of the hidden children, shown after the name when collapsed.
    pub inline: Vec<Icon>,
}

/// Filter popover settings (Blender's defaults).
#[derive(Clone, Debug, PartialEq)]
pub struct Filters {
    pub col_exclude: bool,
    pub col_hide: bool,
    pub col_render: bool,
    pub sort_alpha: bool,
    pub exact: bool,
    pub case_sensitive: bool,
    pub collections: bool,
    pub objects: bool,
    pub contents: bool,
    pub children: bool,
    pub meshes: bool,
    pub cameras: bool,
    pub lights: bool,
    pub empties: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            col_exclude: true,
            col_hide: true,
            col_render: false,
            sort_alpha: true,
            exact: false,
            case_sensitive: false,
            collections: true,
            objects: true,
            contents: true,
            children: true,
            meshes: true,
            cameras: true,
            lights: true,
            empties: true,
        }
    }
}

struct Press {
    key: RowKey,
    at: (i32, i32),
    dragging: bool,
    label: String,
}

pub struct State {
    pub mode: DisplayMode,
    pub search: String,
    pub filters: Filters,
    open: HashMap<String, bool>,
    scroll: i32,
    /// Rows of the last frame, for hotkeys.
    pub rows: Vec<Row>,
    /// Selected rows that are not objects (collections, modifiers, ...).
    pub sel_rows: BTreeSet<RowKey>,
    pub active_row: Option<RowKey>,
    last_click: Option<(RowKey, Instant)>,
    pub renaming: Option<RowKey>,
    rename_start: bool,
    press: Option<Press>,
    show_active: bool,
    body: Rect,
    /// Two clicks on a row within this time rename it.
    pub double_click: Duration,
}

impl Default for State {
    fn default() -> Self {
        State {
            mode: DisplayMode::ViewLayer,
            search: String::new(),
            filters: Filters::default(),
            open: HashMap::new(),
            scroll: 0,
            rows: Vec::new(),
            sel_rows: BTreeSet::new(),
            active_row: None,
            last_click: None,
            renaming: None,
            rename_start: false,
            press: None,
            show_active: false,
            body: Rect::default(),
            double_click: DOUBLE_CLICK,
        }
    }
}

fn object_icon(data: &ObjectData) -> Icon {
    super::properties::object_icon(data)
}
fn data_icon(data: &ObjectData) -> Icon {
    match data {
        ObjectData::Mesh(_) => Icon::Mesh,
        ObjectData::Camera(_) => Icon::CameraData,
        ObjectData::Light(_) => Icon::LightData,
        ObjectData::Empty => Icon::EmptyData,
    }
}

// ---------------------------------------------------------------- tree

struct Build<'a> {
    s: &'a State,
    d: &'a Doc,
    rows: Vec<Row>,
    /// Expand everything (searching).
    all: bool,
}

impl Build<'_> {
    fn is_open(&self, key: &str, default: bool) -> bool {
        self.all || self.s.open.get(key).copied().unwrap_or(default)
    }
    #[allow(clippy::too_many_arguments)]
    fn push(&mut self, key: RowKey, depth: i32, label: String, icon: Icon, has_kids: bool, open_key: String, default_open: bool, owner: Option<usize>, dim: bool) -> bool {
        let open = has_kids && self.is_open(&open_key, default_open);
        self.rows.push(Row { key, depth, label, icon, has_kids, open, open_key, owner, dim, inline: Vec::new() });
        open
    }

    fn type_shown(&self, data: &ObjectData) -> bool {
        let f = &self.s.filters;
        match data {
            ObjectData::Mesh(_) => f.meshes,
            ObjectData::Camera(_) => f.cameras,
            ObjectData::Light(_) => f.lights,
            ObjectData::Empty => f.empties,
        }
    }

    fn sorted(&self, mut v: Vec<usize>) -> Vec<usize> {
        if self.s.filters.sort_alpha {
            v.sort_by(|a, b| self.d.scene.objects[*a].name.cmp(&self.d.scene.objects[*b].name));
        }
        v
    }

    fn collection(&mut self, ci: usize, depth: i32, dim: bool) {
        let sc = &self.d.scene;
        let excluded = dim || self.d.settings.excluded.contains(&ci);
        let kids: Vec<usize> = (0..sc.collections.len()).filter(|&k| sc.collections[k].parent == Some(ci)).collect();
        let objs: Vec<usize> = if self.s.filters.objects {
            (0..sc.objects.len())
                .filter(|&o| {
                    let ob = &sc.objects[o];
                    // Children show under their parent when it is in the same collection.
                    ob.collection == ci && !(self.s.filters.children && ob.parent.is_some_and(|p| p < sc.objects.len() && sc.objects[p].collection == ci)) && self.type_shown(&ob.data)
                })
                .collect()
        } else {
            Vec::new()
        };
        let has = !kids.is_empty() || !objs.is_empty();
        let name = sc.collections[ci].name.clone();
        let open = self.push(RowKey::Col(ci), depth, name.clone(), Icon::Collection, has, format!("col:{name}"), true, None, excluded);
        if !open {
            return;
        }
        for k in kids {
            self.collection(k, depth + 1, excluded);
        }
        for o in self.sorted(objs) {
            self.object(o, depth + 1, excluded);
        }
    }

    fn object(&mut self, o: usize, depth: i32, dim: bool) {
        let sc = &self.d.scene;
        let ob = &sc.objects[o];
        let f = &self.s.filters;
        let children: Vec<usize> = if f.children { (0..sc.objects.len()).filter(|&c| sc.objects[c].parent == Some(o) && sc.objects[c].collection == ob.collection && self.type_shown(&sc.objects[c].data)).collect() } else { Vec::new() };
        let has_data = f.contents && !matches!(ob.data, ObjectData::Empty);
        let has_mods = f.contents && !ob.modifiers.is_empty();
        let mut inline = Vec::new();
        if has_data {
            inline.push(data_icon(&ob.data));
        }
        if has_mods {
            inline.push(Icon::Modifier);
        }
        if f.contents && !ob.materials.is_empty() {
            inline.push(Icon::Material);
        }
        for &c in children.iter().take(3) {
            inline.push(object_icon(&sc.objects[c].data));
        }
        let has = has_data || has_mods || !children.is_empty();
        let hidden = dim || !ob.visible;
        let name = ob.name.clone();
        let open = self.push(RowKey::Obj(o), depth, name.clone(), object_icon(&ob.data), has, format!("obj:{name}"), false, Some(o), hidden);
        if !open {
            if let Some(r) = self.rows.last_mut() {
                r.inline = inline;
            }
            return;
        }
        if has_data {
            let (label, mats) = match ob.data {
                ObjectData::Mesh(mi) => (sc.mesh_names.get(mi).cloned().unwrap_or_default(), ob.materials.clone()),
                _ => (name.clone(), Vec::new()),
            };
            let dopen = self.push(RowKey::Data(o), depth + 1, label, data_icon(&ob.data), !mats.is_empty(), format!("data:{name}"), false, Some(o), hidden);
            if mats.is_empty() {
                if let Some(r) = self.rows.last_mut() {
                    r.has_kids = false;
                }
            } else if !dopen {
                if let Some(r) = self.rows.last_mut() {
                    r.inline = vec![Icon::Material];
                }
            } else {
                for (slot, &mi) in mats.iter().enumerate() {
                    let n = sc.materials.get(mi).map(|m| m.name.clone()).unwrap_or_default();
                    self.push(RowKey::Mat(o, slot), depth + 2, n, Icon::Material, false, String::new(), false, Some(o), hidden);
                }
            }
        }
        if has_mods {
            let mopen = self.push(RowKey::Mods(o), depth + 1, "Modifiers".into(), Icon::Modifier, true, format!("mods:{name}"), false, Some(o), hidden);
            if mopen {
                let shown = self.d.settings.shown_stack(sc, o);
                for (i, (m, live, _)) in shown.iter().enumerate() {
                    let n = if matches!(m, sex_3d::modifiers::Modifier::Subdivision { .. }) { "Subdivision".to_string() } else { m.name().to_string() };
                    self.push(RowKey::Mod(o, i), depth + 2, n, Icon::Modifier, false, String::new(), false, Some(o), hidden || !live);
                }
            } else if let Some(r) = self.rows.last_mut() {
                r.inline = vec![Icon::Modifier; ob.modifiers.len().min(4)];
            }
        }
        for c in self.sorted(children) {
            self.object(c, depth + 1, dim);
        }
    }

    fn view_layer(&mut self, depth: i32) {
        let open = self.push(RowKey::SceneCol, depth, "Scene Collection".into(), Icon::SceneCollection, true, "scenecol".into(), true, None, false);
        if !open {
            return;
        }
        let sc = &self.d.scene;
        if self.s.filters.collections {
            for ci in (0..sc.collections.len()).filter(|&c| sc.collections[c].parent.is_none_or(|p| p >= sc.collections.len())) {
                self.collection(ci, depth + 1, false);
            }
        } else {
            let objs = self.sorted((0..sc.objects.len()).filter(|&o| sc.objects[o].parent.is_none() && self.type_shown(&sc.objects[o].data)).collect());
            for o in objs {
                self.object(o, depth + 1, false);
            }
        }
    }

    fn blender_file(&mut self) {
        let sc = &self.d.scene;
        let open = self.push(RowKey::Cat(0), 0, "Current File".into(), Icon::Output, true, "file".into(), true, None, false);
        if !open {
            return;
        }
        let cams: Vec<usize> = (0..sc.objects.len()).filter(|&o| matches!(sc.objects[o].data, ObjectData::Camera(_))).collect();
        let lights: Vec<usize> = (0..sc.objects.len()).filter(|&o| matches!(sc.objects[o].data, ObjectData::Light(_))).collect();
        if !cams.is_empty() && self.push(RowKey::Cat(1), 1, "Cameras".into(), Icon::CameraData, true, "file:cams".into(), false, None, false) {
            for o in cams {
                self.push(RowKey::Data(o), 2, sc.objects[o].name.clone(), Icon::CameraData, false, String::new(), false, Some(o), false);
            }
        }
        if !sc.collections.is_empty() && self.push(RowKey::Cat(2), 1, "Collections".into(), Icon::Collection, true, "file:cols".into(), false, None, false) {
            for c in 0..sc.collections.len() {
                self.push(RowKey::Col(c), 2, sc.collections[c].name.clone(), Icon::Collection, false, String::new(), false, None, false);
            }
        }
        if !lights.is_empty() && self.push(RowKey::Cat(3), 1, "Lights".into(), Icon::LightData, true, "file:lights".into(), false, None, false) {
            for o in lights {
                self.push(RowKey::Data(o), 2, sc.objects[o].name.clone(), Icon::LightData, false, String::new(), false, Some(o), false);
            }
        }
        if !sc.materials.is_empty() && self.push(RowKey::Cat(4), 1, "Materials".into(), Icon::Material, true, "file:mats".into(), false, None, false) {
            for m in 0..sc.materials.len() {
                self.push(RowKey::MatDb(m), 2, sc.materials[m].name.clone(), Icon::Material, false, String::new(), false, None, false);
            }
        }
        if !sc.meshes.is_empty() && self.push(RowKey::Cat(5), 1, "Meshes".into(), Icon::Mesh, true, "file:meshes".into(), false, None, false) {
            for m in 0..sc.meshes.len() {
                self.push(RowKey::MeshDb(m), 2, sc.mesh_names.get(m).cloned().unwrap_or_default(), Icon::Mesh, false, String::new(), false, None, false);
            }
        }
        if !sc.objects.is_empty() && self.push(RowKey::Cat(6), 1, "Objects".into(), Icon::Object, true, "file:objs".into(), false, None, false) {
            for o in self.sorted((0..sc.objects.len()).collect()) {
                self.push(RowKey::Obj(o), 2, sc.objects[o].name.clone(), object_icon(&sc.objects[o].data), false, String::new(), false, Some(o), !sc.objects[o].visible);
            }
        }
        if self.push(RowKey::Cat(7), 1, "Scenes".into(), Icon::Scene, true, "file:scenes".into(), false, None, false) {
            self.push(RowKey::Scene, 2, sc.name.clone(), Icon::Scene, false, String::new(), false, None, false);
        }
        if self.push(RowKey::Cat(8), 1, "Worlds".into(), Icon::World, true, "file:worlds".into(), false, None, false) {
            self.push(RowKey::World, 2, "World".into(), Icon::World, false, String::new(), false, None, false);
        }
    }

    fn orphans(&mut self) {
        let sc = &self.d.scene;
        let used_mats: BTreeSet<usize> = sc.objects.iter().flat_map(|o| o.materials.iter().copied()).collect();
        let used_meshes: BTreeSet<usize> = sc.objects.iter().filter_map(|o| if let ObjectData::Mesh(m) = o.data { Some(m) } else { None }).collect();
        let mats: Vec<usize> = (0..sc.materials.len()).filter(|m| !used_mats.contains(m)).collect();
        let meshes: Vec<usize> = (0..sc.meshes.len()).filter(|m| !used_meshes.contains(m)).collect();
        if !mats.is_empty() && self.push(RowKey::Cat(4), 0, format!("Materials ({})", mats.len()), Icon::Material, true, "orph:mats".into(), true, None, false) {
            for m in mats {
                self.push(RowKey::MatDb(m), 1, sc.materials[m].name.clone(), Icon::Material, false, String::new(), false, None, false);
            }
        }
        if !meshes.is_empty() && self.push(RowKey::Cat(5), 0, format!("Meshes ({})", meshes.len()), Icon::Mesh, true, "orph:meshes".into(), true, None, false) {
            for m in meshes {
                self.push(RowKey::MeshDb(m), 1, sc.mesh_names.get(m).cloned().unwrap_or_default(), Icon::Mesh, false, String::new(), false, None, false);
            }
        }
    }
}

/// The rows the outliner shows now.
pub fn build_rows(s: &State, d: &Doc) -> Vec<Row> {
    build(s, d, false)
}

fn build(s: &State, d: &Doc, expand_all: bool) -> Vec<Row> {
    let searching = !s.search.trim().is_empty();
    let mut b = Build { s, d, rows: Vec::new(), all: searching || expand_all };
    match s.mode {
        DisplayMode::ViewLayer => b.view_layer(0),
        DisplayMode::Scenes => {
            if b.push(RowKey::Scene, 0, d.scene.name.clone(), Icon::Scene, true, "scene".into(), true, None, false) {
                b.view_layer(1);
                b.push(RowKey::World, 1, "World".into(), Icon::World, false, String::new(), false, None, false);
            }
        }
        DisplayMode::BlenderFile => b.blender_file(),
        DisplayMode::OrphanData => b.orphans(),
        _ => {}
    }
    let mut rows = b.rows;
    if searching {
        // Keep matches and their ancestors (rows are in pre-order).
        let q = s.search.trim();
        let matches = |l: &str| {
            if s.filters.case_sensitive {
                if s.filters.exact { l == q } else { l.contains(q) }
            } else {
                let (l, q) = (l.to_lowercase(), q.to_lowercase());
                if s.filters.exact { l == q } else { l.contains(&q) }
            }
        };
        let mut keep = vec![false; rows.len()];
        let mut flag: Vec<bool> = Vec::new();
        for i in (0..rows.len()).rev() {
            let dpt = rows[i].depth as usize;
            if flag.len() < dpt + 2 {
                flag.resize(dpt + 2, false);
            }
            let child = flag[dpt + 1..].iter().any(|&f| f);
            keep[i] = matches(&rows[i].label) || child;
            for f in &mut flag[dpt + 1..] {
                *f = false;
            }
            flag[dpt] |= keep[i];
        }
        let mut k = keep.into_iter();
        rows.retain(|_| k.next().unwrap_or(false));
    }
    rows
}

// ---------------------------------------------------------------- header

pub fn header(s: &mut State, d: &mut Doc, ui: &mut Ui, hr: Rect) {
    let mr = Rect::new(hr.x, hr.y + 3, 40, hr.h - 6);
    let hov = ui.hovered(mr);
    ui.p.round_rect(mr, 4, if hov { t::BUTTON_DARK_HOVER } else { t::BUTTON_DARK });
    icons::draw(ui.p, s.mode.icon(), mr.x + 13, mr.y + mr.h / 2);
    ui.p.arrow((mr.right() - 9) as f32, (mr.y + mr.h / 2) as f32, 7.0, true, t::TEXT_DIM);
    if ui.clicked(mr) {
        let items = DisplayMode::ALL.iter().map(|m| MenuItem::Check(m.label().into(), *m == s.mode, Op::with("outliner.display_mode", m.label()))).collect();
        ui.open_menu(mr, "Display Mode", items);
    }
    // New Collection and filter buttons at the right.
    let nr = Rect::new(hr.right() - 30, hr.y + 3, 26, hr.h - 6);
    let fr = Rect::new(nr.x - 34, hr.y + 3, 30, hr.h - 6);
    let sr = Rect::new(mr.right() + 6, hr.y + 3, (fr.x - 6 - mr.right() - 6).max(30), hr.h - 6);
    let wid = id("outliner/search");
    ui.p.round_rect(sr, 4, t::BUTTON_DARK);
    let inner = Rect::new(sr.x + 18, sr.y, sr.w - 18, sr.h);
    let editing = matches!(&ui.st.editing, Some((e, _)) if *e == wid);
    let mut text = s.search.clone();
    ui.text_field(wid, inner, &mut text);
    s.search = text;
    if let Some((e, live)) = &ui.st.editing {
        if *e == wid {
            s.search = live.clone();
        }
    }
    icons::draw(ui.p, Icon::Search, sr.x + 10, sr.y + sr.h / 2);
    if !editing && s.search.is_empty() {
        ui.p.label(Rect::new(inner.x + 6, inner.y, inner.w - 6, inner.h), "", t::TEXT_DIM, Align::Left);
    }
    let hov = ui.hovered(fr);
    ui.p.round_rect(fr, 4, if hov { t::BUTTON_DARK_HOVER } else { t::HEADER });
    icons::draw(ui.p, Icon::Funnel, fr.x + 11, fr.y + fr.h / 2);
    ui.p.arrow((fr.right() - 6) as f32, (fr.y + fr.h / 2) as f32, 6.0, true, t::TEXT_DIM);
    if ui.clicked(fr) {
        ui.open_menu(fr, "Filter", filter_menu(&s.filters));
    }
    let hov = ui.hovered(nr);
    if hov {
        ui.p.round_rect(nr, 4, t::BUTTON_DARK_HOVER);
    }
    icons::draw(ui.p, Icon::NewCollection, nr.x + 11, nr.y + nr.h / 2);
    if hov {
        ui.st.hint = "New Collection".into();
    }
    if ui.clicked(nr) {
        new_collection(s, d, None);
    }
}

fn filter_menu(f: &Filters) -> Vec<MenuItem> {
    let c = |l: &str, on: bool, k: &str| MenuItem::Check(l.into(), on, Op::with("outliner.filter_toggle", k));
    vec![
        MenuItem::Heading("Restriction Toggles".into()),
        c("Exclude Checkbox", f.col_exclude, "exclude"),
        c("Hide in Viewport", f.col_hide, "hide"),
        c("Disable in Renders", f.col_render, "render"),
        MenuItem::Separator,
        c("Sort Alphabetically", f.sort_alpha, "sort"),
        MenuItem::Heading("Search".into()),
        c("Exact Match", f.exact, "exact"),
        c("Case Sensitive Matches Only", f.case_sensitive, "case"),
        MenuItem::Heading("Filter".into()),
        c("Collections", f.collections, "collections"),
        c("Objects", f.objects, "objects"),
        c("Object Contents", f.contents, "contents"),
        c("Object Children", f.children, "children"),
        MenuItem::Separator,
        c("Meshes", f.meshes, "meshes"),
        c("Cameras", f.cameras, "cameras"),
        c("Lights", f.lights, "lights"),
        c("Empties", f.empties, "empties"),
    ]
}

// ---------------------------------------------------------------- operations

/// The collection new objects and collections go into: the active one.
fn target_collection(s: &State, d: &Doc) -> Option<usize> {
    match s.active_row {
        Some(RowKey::Col(c)) if c < d.scene.collections.len() => Some(c),
        _ => None,
    }
}

fn new_collection(s: &mut State, d: &mut Doc, parent: Option<usize>) {
    let parent = parent.or_else(|| target_collection(s, d));
    d.push_undo("New Collection");
    let c = d.scene.add_collection("Collection", parent);
    if let Some(p) = parent {
        s.open.insert(format!("col:{}", d.scene.collections[p].name), true);
    }
    s.sel_rows = [RowKey::Col(c)].into();
    s.active_row = Some(RowKey::Col(c));
    d.settings.active_collection = c;
    d.touch();
}

fn subtree(d: &Doc, ci: usize) -> Vec<usize> {
    let mut out = vec![ci];
    let mut i = 0;
    while i < out.len() {
        let c = out[i];
        let kids: Vec<usize> = (0..d.scene.collections.len()).filter(|&k| d.scene.collections[k].parent == Some(c) && !out.contains(&k)).collect();
        out.extend(kids);
        i += 1;
    }
    out
}

fn remove_collection_index(d: &mut Doc, r: usize) {
    d.scene.collections.remove(r);
    for c in &mut d.scene.collections {
        c.parent = match c.parent {
            Some(p) if p == r => None,
            Some(p) if p > r => Some(p - 1),
            p => p,
        };
    }
    for o in &mut d.scene.objects {
        if o.collection > r {
            o.collection -= 1;
        }
    }
    d.settings.collection_removed(r);
}

/// Delete collection `ci`; `hierarchy` also deletes its objects and child
/// collections, otherwise they move to the parent collection.
pub fn delete_collection(s: &mut State, d: &mut Doc, ci: usize, hierarchy: bool) {
    if ci >= d.scene.collections.len() {
        return;
    }
    let tree = subtree(d, ci);
    let parent = d.scene.collections[ci].parent;
    let gone: Vec<usize> = if hierarchy { tree.clone() } else { vec![ci] };
    // Where orphaned objects go: the parent, or another top-level collection.
    let fallback = parent.or_else(|| (0..d.scene.collections.len()).find(|c| !gone.contains(c) && d.scene.collections[*c].parent.is_none()));
    let has_objects = d.scene.objects.iter().any(|o| gone.contains(&o.collection));
    if fallback.is_none() && (!hierarchy && has_objects || d.scene.collections.len() == gone.len()) {
        d.report = "Cannot delete the last collection".into();
        return;
    }
    d.push_undo(if hierarchy { "Delete Hierarchy" } else { "Delete Collection" });
    if hierarchy {
        let mut objs: Vec<usize> = (0..d.scene.objects.len()).filter(|&o| gone.contains(&d.scene.objects[o].collection)).collect();
        objs.sort_unstable();
        for &o in objs.iter().rev() {
            d.scene.remove_object(o);
        }
    } else {
        let to = fallback.unwrap_or(0);
        for o in &mut d.scene.objects {
            if o.collection == ci {
                o.collection = to;
            }
        }
        for c in &mut d.scene.collections {
            if c.parent == Some(ci) {
                c.parent = parent;
            }
        }
    }
    let mut gone = gone;
    gone.sort_unstable();
    for &r in gone.iter().rev() {
        remove_collection_index(d, r);
    }
    d.settings.sync_collections(&mut d.scene);
    s.sel_rows.clear();
    s.active_row = None;
    d.touch();
}

/// Rename whatever `key` shows.
pub fn rename(d: &mut Doc, key: RowKey, want: &str) {
    let want = want.trim();
    if want.is_empty() {
        return;
    }
    match key {
        RowKey::Obj(o) if o < d.scene.objects.len() => super::properties::rename_object(d, o, want),
        RowKey::Col(c) if c < d.scene.collections.len() && d.scene.collections[c].name != want => {
            d.push_undo("Rename");
            d.scene.collections[c].name = sex_3d::scene::unique_name(want, |n| d.scene.collections.iter().enumerate().any(|(i, x)| i != c && x.name == n));
        }
        RowKey::Data(o) if o < d.scene.objects.len() => {
            if let ObjectData::Mesh(mi) = d.scene.objects[o].data {
                rename(d, RowKey::MeshDb(mi), want);
            } else {
                super::properties::rename_object(d, o, want);
            }
        }
        RowKey::MeshDb(mi) if mi < d.scene.mesh_names.len() && d.scene.mesh_names[mi] != want => {
            d.push_undo("Rename");
            d.scene.mesh_names[mi] = sex_3d::scene::unique_name(want, |n| d.scene.mesh_names.iter().enumerate().any(|(i, x)| i != mi && x == n));
        }
        RowKey::Mat(o, slot) => {
            if let Some(&mi) = d.scene.objects.get(o).and_then(|ob| ob.materials.get(slot)) {
                rename(d, RowKey::MatDb(mi), want);
            }
        }
        RowKey::MatDb(mi) if mi < d.scene.materials.len() && d.scene.materials[mi].name != want => {
            d.push_undo("Rename");
            d.scene.materials[mi].name = sex_3d::scene::unique_name(want, |n| d.scene.materials.iter().enumerate().any(|(i, x)| i != mi && x.name == n));
        }
        RowKey::Scene => {
            d.push_undo("Rename");
            d.scene.name = want.to_string();
        }
        _ => return,
    }
    d.touch();
}

fn renamable(key: RowKey) -> bool {
    matches!(key, RowKey::Obj(_) | RowKey::Col(_) | RowKey::Data(_) | RowKey::MeshDb(_) | RowKey::Mat(..) | RowKey::MatDb(_) | RowKey::Scene)
}

/// Start renaming the active row (F2, double-click, context menu).
fn begin_rename(s: &mut State, d: &Doc, key: Option<RowKey>) {
    let key = key.or(s.active_row).or_else(|| d.active().map(RowKey::Obj));
    if let Some(k) = key.filter(|k| renamable(*k)) {
        s.renaming = Some(k);
        s.rename_start = true;
    }
}

/// Toggle viewport visibility (the eye) of a row.
fn toggle_hide(d: &mut Doc, key: RowKey) {
    match key {
        RowKey::Obj(o) if o < d.scene.objects.len() => {
            d.push_undo("Hide");
            let v = &mut d.scene.objects[o].visible;
            *v = !*v;
            if !*v {
                d.scene.selected.retain(|&x| x != o);
            }
        }
        RowKey::Col(c) => {
            d.push_undo("Hide Collection");
            if !d.settings.collection_hidden.remove(&c) {
                d.settings.collection_hidden.insert(c);
            }
            d.settings.sync_collections(&mut d.scene);
        }
        _ => return,
    }
    d.touch();
}

fn toggle_render(d: &mut Doc, key: RowKey) {
    match key {
        RowKey::Obj(o) if o < d.scene.objects.len() => {
            let n = d.scene.objects[o].name.clone();
            if !d.settings.hide_render.remove(&n) {
                d.settings.hide_render.insert(n);
            }
        }
        RowKey::Col(c) => {
            if !d.settings.collection_hide_render.remove(&c) {
                d.settings.collection_hide_render.insert(c);
            }
        }
        _ => return,
    }
    d.dirty = true;
}

fn toggle_exclude(d: &mut Doc, c: usize) {
    d.push_undo("Exclude Collection");
    if !d.settings.excluded.remove(&c) {
        d.settings.excluded.insert(c);
    }
    d.settings.sync_collections(&mut d.scene);
    d.touch();
}

fn objects_in(d: &Doc, ci: usize) -> Vec<usize> {
    let tree = subtree(d, ci);
    (0..d.scene.objects.len()).filter(|&o| tree.contains(&d.scene.objects[o].collection)).collect()
}

fn hierarchy_of(d: &Doc, o: usize) -> Vec<usize> {
    let mut out = vec![o];
    let mut i = 0;
    while i < out.len() {
        let p = out[i];
        let kids: Vec<usize> = (0..d.scene.objects.len()).filter(|&c| d.scene.objects[c].parent == Some(p) && !out.contains(&c)).collect();
        out.extend(kids);
        i += 1;
    }
    out
}

/// Move the selected objects to collection `ci`.
fn move_to_collection(d: &mut Doc, objs: &[usize], ci: usize) {
    if ci >= d.scene.collections.len() || objs.is_empty() {
        return;
    }
    d.push_undo("Move to Collection");
    for &o in objs {
        d.scene.objects[o].collection = ci;
    }
    d.report = format!("Moved {} object(s) to {}", objs.len(), d.scene.collections[ci].name);
    d.touch();
}

/// Run an outliner operator (context menu, header popups, hotkeys).
pub fn run_op(s: &mut State, d: &mut Doc, op: &Op) {
    let arg = op.arg.as_str();
    let idx = arg.parse::<usize>().ok();
    match op.id.as_str() {
        "outliner.display_mode" => {
            if let Some(m) = DisplayMode::ALL.into_iter().find(|m| m.label() == arg) {
                s.mode = m;
                s.scroll = 0;
            }
        }
        "outliner.filter_toggle" => {
            let f = &mut s.filters;
            let b = match arg {
                "exclude" => &mut f.col_exclude,
                "hide" => &mut f.col_hide,
                "render" => &mut f.col_render,
                "sort" => &mut f.sort_alpha,
                "exact" => &mut f.exact,
                "case" => &mut f.case_sensitive,
                "collections" => &mut f.collections,
                "objects" => &mut f.objects,
                "contents" => &mut f.contents,
                "children" => &mut f.children,
                "meshes" => &mut f.meshes,
                "cameras" => &mut f.cameras,
                "lights" => &mut f.lights,
                _ => &mut f.empties,
            };
            *b = !*b;
        }
        "outliner.select_all" => {
            let all = !d.scene.objects.is_empty() && d.scene.selected.len() == d.scene.objects.len();
            let sel = if arg == "DESELECT" || (arg == "TOGGLE" && all) { "DESELECT" } else { "SELECT" };
            s.sel_rows.clear();
            d.run(&Op::with("object.select_all", sel));
        }
        "outliner.object_operation" => {
            let sel = d.scene.selected.clone();
            match arg {
                "DESELECT" => d.scene.selected.clear(),
                "SELECT_HIERARCHY" => {
                    let mut all = Vec::new();
                    for o in sel {
                        for h in hierarchy_of(d, o) {
                            if !all.contains(&h) {
                                all.push(h);
                            }
                        }
                    }
                    d.scene.selected = all;
                }
                _ => {}
            }
            d.touch();
        }
        "outliner.delete" => delete_selected(s, d, false),
        "outliner.delete_hierarchy" => delete_selected(s, d, true),
        "outliner.item_rename" => begin_rename(s, d, None),
        "outliner.hide" => {
            let cols: Vec<usize> = s.sel_rows.iter().filter_map(|k| if let RowKey::Col(c) = k { Some(*c) } else { None }).collect();
            if !cols.is_empty() {
                d.push_undo("Hide");
                d.settings.collection_hidden.extend(cols);
                d.settings.sync_collections(&mut d.scene);
                d.touch();
            }
            if !d.scene.selected.is_empty() {
                d.push_undo("Hide");
                d.run(&Op::new("object.hide_view_set"));
            }
        }
        "outliner.unhide_all" => {
            d.push_undo("Unhide All");
            d.settings.collection_hidden.clear();
            d.settings.sync_collections(&mut d.scene);
            d.run(&Op::new("object.hide_view_clear"));
        }
        "outliner.show_active" => s.show_active = true,
        "outliner.show_hierarchy" => {
            s.scroll = 0;
            for (k, v) in s.open.iter_mut() {
                *v = k.starts_with("col:") || k == "scenecol";
            }
        }
        "outliner.expanded_toggle" | "outliner.show_one_level" => {
            let open = arg != "CLOSE";
            let keys: Vec<(String, i32)> = s.rows.iter().filter(|r| r.has_kids).map(|r| (r.open_key.clone(), r.depth)).collect();
            if open {
                // Open the shallowest closed level.
                if let Some(lvl) = s.rows.iter().filter(|r| r.has_kids && !r.open).map(|r| r.depth).min() {
                    for (k, dp) in keys {
                        if dp == lvl {
                            s.open.insert(k, true);
                        }
                    }
                }
            } else if let Some(lvl) = s.rows.iter().filter(|r| r.has_kids && r.open).map(|r| r.depth).max() {
                for (k, dp) in keys {
                    if dp == lvl {
                        s.open.insert(k, false);
                    }
                }
            }
        }
        "outliner.collection_new" => new_collection(s, d, idx),
        "outliner.collection_delete" => {
            if let Some(c) = idx {
                delete_collection(s, d, c, false)
            }
        }
        "outliner.collection_hierarchy_delete" => {
            if let Some(c) = idx {
                delete_collection(s, d, c, true)
            }
        }
        "outliner.collection_objects_select" | "outliner.collection_objects_deselect" => {
            if let Some(c) = idx {
                let objs = objects_in(d, c);
                if op.id.ends_with("deselect") {
                    d.scene.selected.retain(|o| !objs.contains(o));
                } else {
                    for o in objs {
                        if !d.scene.selected.contains(&o) && d.scene.is_visible(o) {
                            d.scene.selected.push(o);
                        }
                    }
                }
                d.touch();
            }
        }
        "outliner.collection_exclude_set" | "outliner.collection_exclude_clear" => {
            if let Some(c) = idx {
                let want = op.id.ends_with("set");
                if d.settings.excluded.contains(&c) != want {
                    toggle_exclude(d, c);
                }
            }
        }
        "outliner.collection_isolate" => {
            if let Some(c) = idx {
                d.push_undo("Isolate");
                let keep = subtree(d, c);
                let mut chain = vec![];
                let mut p = d.scene.collections.get(c).and_then(|x| x.parent);
                while let Some(pi) = p {
                    if chain.contains(&pi) {
                        break;
                    }
                    chain.push(pi);
                    p = d.scene.collections[pi].parent;
                }
                d.settings.collection_hidden = (0..d.scene.collections.len()).filter(|i| !keep.contains(i) && !chain.contains(i)).collect();
                d.settings.sync_collections(&mut d.scene);
                d.touch();
            }
        }
        "outliner.collection_show" | "outliner.collection_hide" => {
            if let Some(c) = idx {
                let hidden = d.settings.collection_hidden.contains(&c);
                if hidden != op.id.ends_with("hide") {
                    toggle_hide(d, RowKey::Col(c));
                }
            }
        }
        "object.move_to_collection" => {
            if let Some(c) = idx {
                let sel = d.scene.selected.clone();
                move_to_collection(d, &sel, c);
            }
        }
        "outliner.parent_clear" => {
            for o in d.scene.selected.clone() {
                super::properties::set_parent(d, o, None);
            }
        }
        "outliner.orphans_purge" => d.report = "Purging unused data is not available yet".into(),
        _ => d.report = format!("{} is not available yet", op.id),
    }
}

fn delete_selected(s: &mut State, d: &mut Doc, hierarchy: bool) {
    let mut cols: Vec<usize> = s.sel_rows.iter().filter_map(|k| if let RowKey::Col(c) = k { Some(*c) } else { None }).collect();
    if hierarchy && !d.scene.selected.is_empty() {
        let mut all = Vec::new();
        for o in d.scene.selected.clone() {
            for h in hierarchy_of(d, o) {
                if !all.contains(&h) {
                    all.push(h);
                }
            }
        }
        d.scene.selected = all;
    }
    if !d.scene.selected.is_empty() {
        d.run(&Op::new("object.delete"));
    }
    cols.sort_unstable();
    for c in cols.into_iter().rev() {
        delete_collection(s, d, c, hierarchy);
    }
}

fn context_menu(s: &State, d: &Doc, key: Option<RowKey>) -> (String, Vec<MenuItem>) {
    let o = |l: &str, k: &str, id: &str, a: &str| MenuItem::op(l, k, Op::with(id, a));
    let colls: Vec<MenuItem> = d.scene.collections.iter().enumerate().map(|(i, c)| o(&c.name, "", "object.move_to_collection", &i.to_string())).collect();
    match key {
        Some(RowKey::Col(c)) => {
            let a = c.to_string();
            let a = a.as_str();
            (
                "Collection".into(),
                vec![
                    o("New Collection", "", "outliner.collection_new", a),
                    MenuItem::Separator,
                    o("Delete Hierarchy", "", "outliner.collection_hierarchy_delete", a),
                    o("Delete", "X", "outliner.collection_delete", a),
                    MenuItem::Separator,
                    o("Select Objects", "", "outliner.collection_objects_select", a),
                    o("Deselect Objects", "", "outliner.collection_objects_deselect", a),
                    MenuItem::Separator,
                    o("Rename", "F2", "outliner.item_rename", ""),
                    MenuItem::Sub(
                        "Visibility".into(),
                        vec![
                            o("Isolate", "", "outliner.collection_isolate", a),
                            MenuItem::Separator,
                            o("Show", "", "outliner.collection_show", a),
                            o("Hide", "", "outliner.collection_hide", a),
                            MenuItem::Separator,
                            o("Exclude from View Layer", "", "outliner.collection_exclude_set", a),
                            o("Include in View Layer", "", "outliner.collection_exclude_clear", a),
                        ],
                    ),
                ],
            )
        }
        Some(RowKey::Obj(_)) | Some(RowKey::Data(_)) | Some(RowKey::Mat(..)) | Some(RowKey::Mods(_)) | Some(RowKey::Mod(..)) => (
            "Object".into(),
            vec![
                o("Select", "", "outliner.object_operation", "SELECT"),
                o("Select Hierarchy", "", "outliner.object_operation", "SELECT_HIERARCHY"),
                o("Deselect", "", "outliner.object_operation", "DESELECT"),
                MenuItem::Separator,
                o("Delete", "X", "outliner.delete", ""),
                o("Delete Hierarchy", "", "outliner.delete_hierarchy", ""),
                o("Duplicate", "Shift D", "object.duplicate", ""),
                MenuItem::Separator,
                o("Rename", "F2", "outliner.item_rename", ""),
                MenuItem::Separator,
                o("Hide", "H", "outliner.hide", ""),
                o("Show All", "Alt H", "outliner.unhide_all", ""),
                MenuItem::Separator,
                MenuItem::Sub("Move to Collection".into(), colls),
                o("Clear Parent", "", "outliner.parent_clear", ""),
                o("New Collection", "", "outliner.collection_new", ""),
            ],
        ),
        _ => {
            let _ = s;
            (
                String::new(),
                vec![
                    o("New Collection", "", "outliner.collection_new", ""),
                    MenuItem::Separator,
                    o("Show/Hide One Level", "", "outliner.show_one_level", ""),
                    o("Show Hierarchy", "Home", "outliner.show_hierarchy", ""),
                    o("Show Active", "Period", "outliner.show_active", ""),
                    MenuItem::Separator,
                    o("Show All", "Alt H", "outliner.unhide_all", ""),
                ],
            )
        }
    }
}

// ---------------------------------------------------------------- body

fn row_selected(s: &State, d: &Doc, r: &Row) -> bool {
    match r.key {
        RowKey::Obj(o) => d.is_selected(o),
        k => s.sel_rows.contains(&k),
    }
}
fn row_active(s: &State, d: &Doc, r: &Row) -> bool {
    match r.key {
        RowKey::Obj(o) => d.scene.active_object == Some(o) && s.active_row.is_none_or(|k| matches!(k, RowKey::Obj(_))),
        k => s.active_row == Some(k),
    }
}

/// Click on a row: select like Blender (Ctrl extends, Shift selects a range).
fn click_row(s: &mut State, d: &mut Doc, i: usize, mods: Mods) {
    let r = s.rows[i].clone();
    if mods.shift && !mods.ctrl {
        // Range from the active row.
        let from = s.rows.iter().position(|x| row_active(s, d, x)).unwrap_or(i);
        let (a, b) = if from <= i { (from, i) } else { (i, from) };
        for k in a..=b {
            if let RowKey::Obj(o) = s.rows[k].key {
                if !d.scene.selected.contains(&o) {
                    d.scene.selected.push(o);
                }
            }
        }
        if let RowKey::Obj(o) = r.key {
            d.scene.active_object = Some(o);
        }
        d.touch();
        return;
    }
    match r.key {
        RowKey::Obj(o) => {
            s.active_row = None;
            if mods.ctrl {
                d.toggle_select(o);
            } else {
                s.sel_rows.clear();
                d.select_only(Some(o));
            }
        }
        RowKey::Col(c) => {
            if !mods.ctrl {
                s.sel_rows.clear();
                d.scene.selected.clear();
                d.touch();
            }
            s.sel_rows.insert(r.key);
            s.active_row = Some(r.key);
            d.settings.active_collection = c;
        }
        k => {
            if !mods.ctrl {
                s.sel_rows.clear();
            }
            s.sel_rows.insert(k);
            s.active_row = Some(k);
            if let Some(o) = r.owner {
                if mods.ctrl {
                    if !d.scene.selected.contains(&o) {
                        d.scene.selected.push(o);
                    }
                    d.scene.active_object = Some(o);
                    d.touch();
                } else {
                    d.select_only(Some(o));
                }
            }
        }
    }
}

fn drop_on(s: &mut State, d: &mut Doc, dragged: RowKey, target: RowKey) {
    match (dragged, target) {
        (RowKey::Obj(o), RowKey::Col(c)) => {
            let mut objs = d.scene.selected.clone();
            if !objs.contains(&o) {
                objs = vec![o];
            }
            move_to_collection(d, &objs, c);
        }
        (RowKey::Obj(_), RowKey::SceneCol) => d.report = "Objects live in collections; drop onto a collection".into(),
        (RowKey::Obj(o), RowKey::Obj(p)) if o != p => {
            let mut objs = d.scene.selected.clone();
            if !objs.contains(&o) {
                objs = vec![o];
            }
            for c in objs.into_iter().filter(|&c| c != p) {
                super::properties::set_parent(d, c, Some(p));
            }
            let pname = d.scene.objects[p].name.clone();
            s.open.insert(format!("obj:{pname}"), true);
            d.report = format!("Parented to {pname}");
        }
        (RowKey::Col(c), RowKey::Col(p)) if c != p => {
            if subtree(d, c).contains(&p) {
                d.report = "Cannot move a collection into itself".into();
                return;
            }
            d.push_undo("Move Collection");
            d.scene.collections[c].parent = Some(p);
            d.settings.sync_collections(&mut d.scene);
            d.touch();
        }
        (RowKey::Col(c), RowKey::SceneCol) => {
            d.push_undo("Move Collection");
            d.scene.collections[c].parent = None;
            d.touch();
        }
        _ => {}
    }
}

/// Run outliner operators queued by this frame's menus.
fn run_queued(s: &mut State, d: &mut Doc) {
    let mine = |op: &Op| op.id.starts_with("outliner.") || op.id == "object.move_to_collection";
    if !d.queue.iter().any(mine) {
        return;
    }
    let (ops, rest): (Vec<Op>, Vec<Op>) = core::mem::take(&mut d.queue).into_iter().partition(mine);
    d.queue = rest;
    for op in &ops {
        run_op(s, d, op);
    }
    // Ask the window to draw again so every editor shows the change.
    d.queue.push(Op::with("screen.frame_offset", "0"));
}

pub fn body(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect) {
    if r.is_empty() {
        return;
    }
    s.body = r;
    let gen0 = d.generation;
    run_queued(s, d);
    if s.show_active {
        reveal_active(s, d);
    }
    s.rows = build_rows(s, d);
    if matches!(s.mode, DisplayMode::VideoSequencer | DisplayMode::DataApi | DisplayMode::LibraryOverrides) {
        ui.p.fill(r, t::OUTLINER);
        super::note(ui, r, &format!("{} is not available yet", s.mode.label()));
        return;
    }
    let content_h = s.rows.len() as i32 * ROW_H + ROW_H;
    let max = (content_h - r.h).max(0);
    if s.show_active {
        s.show_active = false;
        if let Some(i) = d.active().and_then(|a| s.rows.iter().position(|x| x.key == RowKey::Obj(a))) {
            let y = i as i32 * ROW_H;
            if y < s.scroll || y + ROW_H > s.scroll + r.h {
                s.scroll = y - r.h / 2;
            }
        }
    }
    if let Some(Input::Wheel { x, y, steps, .. }) = ui.input {
        if r.contains(x, y) {
            ui.input = None;
            s.scroll -= steps * ROW_H * 2;
        }
    }
    s.scroll = s.scroll.clamp(0, max);

    // Restriction columns, right to left.
    let f = &s.filters;
    let mut cols: Vec<u8> = Vec::new();
    if f.col_exclude {
        cols.push(0);
    }
    if f.col_hide {
        cols.push(1);
    }
    if f.col_render {
        cols.push(2);
    }
    let cols_x = r.right() - 6 - cols.len() as i32 * COL_W;
    let col_rect = |ci: usize, y: i32| Rect::new(cols_x + ci as i32 * COL_W, y, COL_W, ROW_H);

    let mouse = ui.mouse();
    let drop_target = s.press.as_ref().filter(|p| p.dragging).and_then(|_| {
        if r.contains(mouse.0, mouse.1) {
            let i = ((mouse.1 - r.y + s.scroll) / ROW_H) as usize;
            s.rows.get(i).map(|x| x.key)
        } else {
            None
        }
    });

    let rename_id = id("outliner/rename");
    let open_before = s.open.len();
    let mut clicked_row: Option<(usize, Mods)> = None;
    let mut toggle: Option<(u8, RowKey)> = None;
    let mut disclose: Option<(usize, bool)> = None;
    let mut menu_for: Option<(Option<RowKey>, i32, i32)> = None;
    let rows = s.rows.clone();
    let mut y = r.y - s.scroll;
    for (i, row) in rows.iter().enumerate() {
        let rr = Rect::new(r.x, y, r.w, ROW_H);
        y += ROW_H;
        if rr.bottom() < r.y || rr.y > r.bottom() {
            continue;
        }
        let sel = row_selected(s, d, row);
        let act = row_active(s, d, row);
        ui.p.fill(rr, if i % 2 == 1 { t::OUTLINER_ALT } else { t::OUTLINER });
        if act {
            ui.p.fill(rr, 0x4b6ca8);
        } else if sel {
            ui.p.fill(rr, t::OUTLINER_SEL);
        } else if ui.hovered(rr) {
            ui.p.fill_alpha(rr, 0xffffff, 10);
        }
        if drop_target == Some(row.key) {
            ui.p.round_frame(rr.shrink(1), 3, t::SELECTED_HOVER);
        }
        let x0 = r.x + 6 + row.depth * INDENT;
        if row.has_kids {
            let ar = Rect::new(x0 - 2, rr.y, 16, ROW_H);
            ui.p.arrow((x0 + 6) as f32, (rr.y + ROW_H / 2) as f32, 8.0, row.open, if ui.hovered(ar) { t::TEXT_HI } else { t::TEXT_DIM });
            if let Some(Input::Down { x, y: py, button: Button::Left, mods }) = ui.input {
                if ar.contains(x, py) && r.contains(x, py) {
                    ui.input = None;
                    disclose = Some((i, mods.shift));
                }
            }
        }
        let ix = x0 + 22;
        let dim = row.dim;
        icons::draw_tinted(ui.p, row.icon, ix, rr.y + ROW_H / 2, dim.then_some(0x7a7a7a));
        let tx = ix + 12;
        let name_w = cols_x - tx - 6;
        if s.renaming == Some(row.key) {
            if s.rename_start {
                s.rename_start = false;
                ui.st.editing = Some((rename_id, row.label.clone()));
                ui.st.edit_select_all = true;
            }
            let mut text = row.label.clone();
            let fr = Rect::new(tx, rr.y + 1, name_w.max(40), ROW_H - 2);
            if ui.text_field(rename_id, fr, &mut text) {
                rename(d, row.key, &text);
            }
            if !matches!(&ui.st.editing, Some((e, _)) if *e == rename_id) {
                s.renaming = None;
            }
        } else {
            let color = if dim {
                t::TEXT_DIM
            } else if act {
                t::TEXT_HI
            } else if matches!(row.key, RowKey::Obj(_)) && sel {
                0xffaf29
            } else {
                t::TEXT
            };
            let tw = ui.p.text_width(&row.label).min(name_w.max(0));
            ui.p.label(Rect::new(tx, rr.y, name_w.max(0), ROW_H), &row.label, color, Align::Left);
            // Collapsed children as small icons after the name.
            let mut cx = tx + tw + 14;
            for ic in &row.inline {
                if cx + 8 > cols_x {
                    break;
                }
                icons::draw_tinted(ui.p, *ic, cx, rr.y + ROW_H / 2, dim.then_some(0x7a7a7a));
                cx += 18;
            }
        }
        // Restriction toggles.
        for (ci, &c) in cols.iter().enumerate() {
            let cr = col_rect(ci, rr.y);
            let (cx, cy) = (cr.x + cr.w / 2, cr.y + ROW_H / 2);
            let hov = ui.hovered(cr);
            match (c, row.key) {
                (0, RowKey::Col(k)) => {
                    let on = !d.settings.excluded.contains(&k);
                    let b = Rect::new(cx - 6, cy - 6, 13, 13);
                    ui.p.round_rect(b, 3, if on { t::SELECTED } else if hov { t::BUTTON_HOVER } else { t::BUTTON });
                    if on {
                        ui.p.line((b.x + 3) as f32, (b.y + 7) as f32, (b.x + 5) as f32, (b.y + 10) as f32, t::TEXT_HI);
                        ui.p.line((b.x + 5) as f32, (b.y + 10) as f32, (b.x + 10) as f32, (b.y + 3) as f32, t::TEXT_HI);
                    }
                }
                (1, RowKey::Col(k)) => {
                    let shown = !d.settings.collection_hidden.contains(&k);
                    icons::draw_tinted(ui.p, if shown { Icon::Eye } else { Icon::EyeClosed }, cx, cy, (row.dim && shown).then_some(0x6a6a6a));
                }
                (1, RowKey::Obj(o)) => {
                    let shown = d.scene.objects[o].visible;
                    icons::draw_tinted(ui.p, if shown { Icon::Eye } else { Icon::EyeClosed }, cx, cy, (row.dim && shown).then_some(0x6a6a6a));
                }
                (2, RowKey::Col(k)) => icons::draw(ui.p, if d.settings.collection_hide_render.contains(&k) { Icon::RenderOff } else { Icon::RenderOn }, cx, cy),
                (2, RowKey::Obj(o)) => icons::draw(ui.p, if d.settings.hide_render.contains(&d.scene.objects[o].name) { Icon::RenderOff } else { Icon::RenderOn }, cx, cy),
                _ => continue,
            }
            if ui.clicked(cr) {
                toggle = Some((c, row.key));
            }
        }
        // Row clicks.
        match ui.input {
            Some(Input::Down { x, y: py, button: Button::Left, mods }) | Some(Input::Down { x, y: py, button: Button::Middle, mods: mods @ Mods { ctrl: true, .. } }) if rr.contains(x, py) && r.contains(x, py) => {
                ui.input = None;
                clicked_row = Some((i, mods));
            }
            Some(Input::Down { x, y: py, button: Button::Right, .. }) if rr.contains(x, py) && r.contains(x, py) => {
                ui.input = None;
                menu_for = Some((Some(row.key), x, py));
                if !row_selected(s, d, row) {
                    click_row(s, d, i, Mods::NONE);
                }
            }
            _ => {}
        }
    }
    // Empty space below the rows.
    match ui.input {
        Some(Input::Down { x, y: py, button: Button::Left, mods }) if r.contains(x, py) && !mods.ctrl && !mods.shift => {
            ui.input = None;
            s.sel_rows.clear();
            s.active_row = None;
            d.select_only(None);
        }
        Some(Input::Down { x, y: py, button: Button::Right, .. }) if r.contains(x, py) => {
            ui.input = None;
            menu_for = Some((None, x, py));
        }
        _ => {}
    }

    if let Some((c, key)) = toggle {
        match (c, key) {
            (0, RowKey::Col(k)) => toggle_exclude(d, k),
            (1, k) => toggle_hide(d, k),
            (2, k) => toggle_render(d, k),
            _ => {}
        }
    }
    if let Some((i, recursive)) = disclose {
        let row = &s.rows[i];
        let open = !row.open;
        s.open.insert(row.open_key.clone(), open);
        if recursive {
            // Shift+click: the whole branch, including rows not shown yet.
            let key = row.key;
            let all = build(s, d, true);
            if let Some(j) = all.iter().position(|x| x.key == key) {
                let dpt = all[j].depth;
                for k in all[j + 1..].iter().take_while(|x| x.depth > dpt) {
                    if k.has_kids {
                        s.open.insert(k.open_key.clone(), open);
                    }
                }
            }
        }
    }
    if let Some((i, mods)) = clicked_row {
        let key = s.rows[i].key;
        let now = Instant::now();
        let double = !mods.ctrl && !mods.shift && s.last_click.as_ref().is_some_and(|(k, at)| *k == key && now.duration_since(*at) < s.double_click);
        if double {
            s.last_click = None;
            begin_rename(s, d, Some(key));
        } else {
            s.last_click = Some((key, now));
            click_row(s, d, i, mods);
            let (mx, my) = ui.mouse();
            s.press = Some(Press { key, at: (mx, my), dragging: false, label: s.rows[i].label.clone() });
        }
    }
    if let Some((key, x, y)) = menu_for {
        let (title, items) = context_menu(s, d, key);
        let mut pop = crate::ui::Popup::new(&title, items, x, y);
        pop.owner = Rect::new(x, y, 1, 1);
        ui.st.open_popup = Some(pop);
    }

    // Dragging rows onto other rows.
    match ui.input {
        Some(Input::Move { x, y }) => {
            if let Some(p) = &mut s.press {
                if !p.dragging && ((x - p.at.0).abs() > 4 || (y - p.at.1).abs() > 4) && matches!(p.key, RowKey::Obj(_) | RowKey::Col(_)) {
                    p.dragging = true;
                }
                if p.dragging {
                    ui.input = None;
                }
            }
        }
        Some(Input::Up { button: Button::Left, .. }) => {
            if let Some(p) = s.press.take() {
                if p.dragging {
                    ui.input = None;
                    if let Some(target) = drop_target {
                        drop_on(s, d, p.key, target);
                    }
                }
            }
        }
        _ => {}
    }
    if let Some(p) = &s.press {
        if p.dragging {
            let (mx, my) = ui.mouse();
            let label = if drop_target.is_some() { format!("{}  ->  drop", p.label) } else { p.label.clone() };
            let w = ui.p.text_width(&label) + 16;
            let gr = Rect::new(mx + 12, my + 4, w, 20);
            ui.p.round_rect_alpha(gr, 4, 0x000000, 160);
            ui.p.label(gr, &label, t::TEXT_HI, Align::Center);
        }
    }
    // Rows were built before this frame's clicks: draw again if they changed.
    let changed = d.generation != gen0 || disclose.is_some() || toggle.is_some() || clicked_row.is_some() || s.open.len() != open_before;
    if changed && !d.queue.iter().any(|o| o.id == "screen.frame_offset" && o.arg == "0") {
        d.queue.push(Op::with("screen.frame_offset", "0"));
    }
    // Scrollbar.
    if max > 0 {
        let bh = ((r.h as f32 * r.h as f32 / content_h as f32) as i32).max(20);
        let by = r.y + ((r.h - bh) as f32 * s.scroll as f32 / max as f32) as i32;
        ui.p.round_rect(Rect::new(r.right() - 4, by, 3, bh), 1, 0x5a5a5a);
    }
}

/// Expand the active object's ancestors (Numpad . / period, "Show Active").
fn reveal_active(s: &mut State, d: &Doc) {
    let Some(a) = d.active() else { return };
    let mut p = d.scene.objects[a].parent;
    let mut guard = 0;
    while let Some(pi) = p {
        s.open.insert(format!("obj:{}", d.scene.objects[pi].name), true);
        p = d.scene.objects[pi].parent;
        guard += 1;
        if guard > d.scene.objects.len() {
            break;
        }
    }
    let mut c = Some(d.scene.objects[a].collection);
    guard = 0;
    while let Some(ci) = c.filter(|&ci| ci < d.scene.collections.len()) {
        s.open.insert(format!("col:{}", d.scene.collections[ci].name), true);
        c = d.scene.collections[ci].parent;
        guard += 1;
        if guard > d.scene.collections.len() {
            break;
        }
    }
    s.open.insert("scenecol".into(), true);
}

pub fn hotkey(s: &mut State, d: &mut Doc, input: &Input, _r: Rect) -> bool {
    let Input::Key { key, mods, .. } = *input else { return false };
    let op = match (key, mods) {
        (Key::Char('a'), Mods::NONE) => Op::with("outliner.select_all", "TOGGLE"),
        (Key::Char('a'), Mods::ALT) => Op::with("outliner.select_all", "DESELECT"),
        (Key::Char('x'), Mods::NONE) | (Key::Delete, Mods::NONE) => Op::new("outliner.delete"),
        (Key::Char('h'), Mods::NONE) => Op::new("outliner.hide"),
        (Key::Char('h'), Mods::ALT) | (Key::Char('h'), Mods::SHIFT) => Op::new("outliner.unhide_all"),
        (Key::F(2), _) => Op::new("outliner.item_rename"),
        (Key::NumDot, _) | (Key::Char('.'), Mods::NONE) => Op::new("outliner.show_active"),
        (Key::Home, _) => Op::new("outliner.show_hierarchy"),
        (Key::NumPlus, _) | (Key::Char('+'), _) | (Key::Char('='), _) => Op::with("outliner.show_one_level", "OPEN"),
        (Key::NumMinus, _) | (Key::Char('-'), _) => Op::with("outliner.show_one_level", "CLOSE"),
        (Key::Char('c'), Mods::NONE) => Op::new("outliner.collection_new"),
        (Key::Char('d'), Mods::SHIFT) => Op::new("object.duplicate"),
        (Key::Char('f'), Mods::CTRL) => {
            s.search.clear();
            return true;
        }
        (Key::Esc, _) if s.press.as_ref().is_some_and(|p| p.dragging) => {
            s.press = None;
            return true;
        }
        _ => return false,
    };
    if op.id == "object.duplicate" {
        d.queue.push(op);
    } else {
        run_op(s, d, &op);
        // Ask the window to draw again so the change shows.
        d.queue.push(Op::with("screen.frame_offset", "0"));
    }
    true
}

#[cfg(test)]
mod tests;
