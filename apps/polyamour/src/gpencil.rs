// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil in Polyamour: the tool settings of Draw, Edit, Sculpt,
//! Weight Paint and Vertex Paint on a Grease Pencil object, the
//! `grease_pencil.*` operators (layers, materials, strokes, frames,
//! interpolation, modifiers and visual effects), the Add menu entries and
//! the evaluated-strokes cache the viewport draws from. The geometry and
//! drawing work is `sex_3d::grease_pencil`; the viewport side is
//! `editors/view3d/gpview.rs`.
pub mod panels;
#[cfg(test)]
mod tests;
pub mod ui;

use crate::doc::{Doc, Mode};
use crate::ui::{MenuItem, Op};
use sex_3d::grease_pencil::edit::{self as gedit, SelectAction, SelectMode};
use sex_3d::grease_pencil::effects::GpEffect;
use sex_3d::grease_pencil::geom::SmoothWhat;
use sex_3d::grease_pencil::interp::{self, Easing};
use sex_3d::grease_pencil::lineart::LineArtSource;
use sex_3d::grease_pencil::modifiers::{self, evaluate, EvalContext, EvalLayer, GpModKind, GpModifier};
use sex_3d::grease_pencil::sculpt::{SculptTool, VertexTool, WeightTool};
use sex_3d::grease_pencil::tools::{DrawBrush, EraseMode, FillSettings, Primitive};
use sex_3d::grease_pencil::view::{DrawPlane, Placement};
use sex_3d::grease_pencil::{self as gp, layer_world_matrix, CapType, Drawing, GpLayer, GpMaterial, GpStroke, GreasePencil, KeyframeType, LayerGroup};
use sex_3d::scene::{Object, ObjectData};
use sex_3d::Mat4;

/// The Draw Mode tools (toolbar order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawTool {
    Draw,
    Fill,
    Erase,
    Tint,
    Eyedropper,
    Primitive(Primitive),
}

impl DrawTool {
    pub const ALL: [DrawTool; 11] = [
        DrawTool::Draw,
        DrawTool::Fill,
        DrawTool::Erase,
        DrawTool::Tint,
        DrawTool::Eyedropper,
        DrawTool::Primitive(Primitive::Line),
        DrawTool::Primitive(Primitive::Polyline),
        DrawTool::Primitive(Primitive::Arc),
        DrawTool::Primitive(Primitive::Curve),
        DrawTool::Primitive(Primitive::Box),
        DrawTool::Primitive(Primitive::Circle),
    ];
    pub fn label(self) -> &'static str {
        match self {
            DrawTool::Draw => "Draw",
            DrawTool::Fill => "Fill",
            DrawTool::Erase => "Erase",
            DrawTool::Tint => "Tint",
            DrawTool::Eyedropper => "Eyedropper",
            DrawTool::Primitive(p) => p.label(),
        }
    }
    pub fn from_label(s: &str) -> Option<DrawTool> {
        DrawTool::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(s))
    }
}

/// Edit Mode tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditTool {
    SelectBox,
    Cursor,
    Extrude,
    Radius,
}

impl EditTool {
    pub const ALL: [EditTool; 4] = [EditTool::SelectBox, EditTool::Cursor, EditTool::Extrude, EditTool::Radius];
    pub fn label(self) -> &'static str {
        match self {
            EditTool::SelectBox => "Select Box",
            EditTool::Cursor => "Cursor",
            EditTool::Extrude => "Extrude",
            EditTool::Radius => "Radius",
        }
    }
}

/// One brush mode's size and strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushSize {
    pub radius: f32,
    pub strength: f32,
}

/// Grease Pencil tool settings and caches (the document's `gp`).
#[derive(Clone, Debug)]
pub struct GpState {
    pub tool: DrawTool,
    pub edit_tool: EditTool,
    pub brush: DrawBrush,
    pub plane: DrawPlane,
    pub placement: Placement,
    /// New keys copy the held drawing (Blender's Additive Drawing).
    pub additive: bool,
    pub erase_mode: EraseMode,
    pub erase: BrushSize,
    pub fill: FillSettings,
    pub tint: BrushSize,
    pub tint_color: [f32; 3],
    pub primitive_subdiv: u32,
    pub sculpt_tool: SculptTool,
    pub sculpt: BrushSize,
    pub vertex_tool: VertexTool,
    pub vertex: BrushSize,
    pub vertex_color: [f32; 3],
    /// Vertex Paint mode: stroke, fill or both.
    pub vertex_fill: bool,
    pub vertex_stroke: bool,
    pub weight_tool: WeightTool,
    pub weight: BrushSize,
    pub weight_value: f32,
    pub select_mode: SelectMode,
    pub easing: Easing,
    pub interp_step: i32,
    /// Copied strokes (Ctrl+C) and the Clone brush's source.
    pub clipboard: Vec<GpStroke>,
    /// Evaluated layers by object, valid for (generation, frame).
    pub cache: Option<(u64, i32, Vec<(usize, Vec<EvalLayer>)>)>,
}

impl Default for GpState {
    fn default() -> Self {
        GpState {
            tool: DrawTool::Draw,
            edit_tool: EditTool::SelectBox,
            brush: DrawBrush::default(),
            plane: DrawPlane::Front,
            placement: Placement::Origin,
            additive: false,
            erase_mode: EraseMode::Point,
            erase: BrushSize { radius: 20.0, strength: 1.0 },
            fill: FillSettings::default(),
            tint: BrushSize { radius: 25.0, strength: 0.5 },
            tint_color: [1.0, 0.2, 0.2],
            primitive_subdiv: 6,
            sculpt_tool: SculptTool::Smooth,
            sculpt: BrushSize { radius: 40.0, strength: 0.5 },
            vertex_tool: VertexTool::Draw,
            vertex: BrushSize { radius: 30.0, strength: 1.0 },
            vertex_color: [1.0, 1.0, 1.0],
            vertex_fill: false,
            vertex_stroke: true,
            weight_tool: WeightTool::Draw,
            weight: BrushSize { radius: 30.0, strength: 0.8 },
            weight_value: 1.0,
            select_mode: SelectMode::Point,
            easing: Easing::Linear,
            interp_step: 1,
            clipboard: Vec::new(),
            cache: None,
        }
    }
}

impl GpState {
    /// Size and strength of the brush the current mode uses.
    pub fn size_mut(&mut self, mode: Mode) -> &mut BrushSize {
        match mode {
            Mode::Sculpt => &mut self.sculpt,
            Mode::VertexPaint => &mut self.vertex,
            Mode::WeightPaint => &mut self.weight,
            _ => match self.tool {
                DrawTool::Erase => &mut self.erase,
                _ => &mut self.tint,
            },
        }
    }
}

/// The active object if it is a Grease Pencil object: (object, data).
pub fn active_gp(d: &Doc) -> Option<(usize, usize)> {
    let a = d.active()?;
    match d.scene.objects[a].data {
        ObjectData::GreasePencil(g) if g < d.scene.grease_pencils.len() => Some((a, g)),
        _ => None,
    }
}

/// True in a Grease Pencil mode (Draw, Edit, Sculpt, Weight or Vertex Paint
/// on a Grease Pencil object).
pub fn in_mode(d: &Doc) -> bool {
    active_gp(d).is_some() && matches!(d.mode, Mode::Draw | Mode::Edit | Mode::Sculpt | Mode::WeightPaint | Mode::VertexPaint)
}

/// The modes a Grease Pencil object offers.
pub const MODES: [Mode; 6] = [Mode::Object, Mode::Edit, Mode::Sculpt, Mode::Draw, Mode::WeightPaint, Mode::VertexPaint];

/// Layers that edit operators change: visible and unlocked, with a key
/// shown at the current frame. Returns (layer, frame index, world matrix).
pub fn editable(d: &Doc, oi: usize, g: usize) -> Vec<(usize, usize, Mat4)> {
    let gpd = &d.scene.grease_pencils[g];
    (0..gpd.layers.len())
        .filter(|&li| gpd.layer_visible(li) && !gpd.layer_locked(li))
        .filter_map(|li| gpd.layers[li].frame_index_at(d.frame).map(|fi| (li, fi, layer_world_matrix(&d.scene, oi, gpd, li))))
        .collect()
}

/// Run `f` on every editable drawing of the active object; true if any
/// call returned true.
pub fn each_drawing(d: &mut Doc, mut f: impl FnMut(&mut Drawing, &Mat4) -> bool) -> bool {
    let Some((oi, g)) = active_gp(d) else { return false };
    let mut any = false;
    for (li, fi, m) in editable(d, oi, g) {
        any |= f(&mut d.scene.grease_pencils[g].layers[li].frames[fi].drawing, &m);
    }
    any
}

/// Evaluated layers of object `oi` for the viewport (cached per scene
/// generation and frame).
pub fn evaluated(d: &mut Doc) -> Vec<(usize, Vec<EvalLayer>)> {
    let key = (d.generation, d.frame);
    if let Some((g, f, v)) = &d.gp.cache {
        if (*g, *f) == key {
            return v.clone();
        }
    }
    let mut out = Vec::new();
    for (oi, o) in d.scene.objects.iter().enumerate() {
        if let ObjectData::GreasePencil(g) = o.data {
            if let Some(gpd) = d.scene.grease_pencils.get(g) {
                out.push((oi, evaluate(gpd, &o.gp_modifiers, &EvalContext { scene: Some(&d.scene), owner: oi, frame: d.frame, render: false })));
            }
        }
    }
    d.gp.cache = Some((key.0, key.1, out.clone()));
    out
}

// ---------------------------------------------------------------- adding

/// `object.grease_pencil_add`: EMPTY, STROKE, MONKEY (our cat head),
/// LINEART_SCENE, LINEART_COLLECTION and LINEART_OBJECT.
pub fn add_object(d: &mut Doc, kind: &str) {
    let (name, data, lineart) = match kind {
        "STROKE" => ("Stroke", gp::sample_stroke(), None),
        "MONKEY" | "CAT" => ("Cat Head", gp::sample_cat(), None),
        "LINEART_SCENE" => ("Line Art", lineart_data(), Some(LineArtSource::Scene)),
        "LINEART_COLLECTION" => {
            let c = d.active().map(|a| d.scene.objects[a].collection).and_then(|c| d.scene.collections.get(c)).map(|c| c.name.clone()).unwrap_or_default();
            ("Line Art", lineart_data(), Some(LineArtSource::Collection(c)))
        }
        "LINEART_OBJECT" => {
            let Some(a) = d.active().filter(|&a| matches!(d.scene.objects[a].data, ObjectData::Mesh(_))) else {
                d.report = "Object Line Art needs an active mesh object".into();
                return;
            };
            let n = d.scene.objects[a].name.clone();
            ("Line Art", lineart_data(), Some(LineArtSource::Object(n)))
        }
        _ => ("GPencil", GreasePencil::blank(), None),
    };
    d.push_undo(&format!("Add {name}"));
    let gi = d.scene.add_grease_pencil(name, data);
    let mut o = Object::new(name, ObjectData::GreasePencil(gi));
    if let Some(src) = lineart {
        let mut m = GpModifier::new("LINEART").expect("line art");
        if let GpModKind::LineArt(s) = &mut m.kind {
            s.source = src;
        }
        o.gp_modifiers.push(m);
    } else {
        o.transform.location = d.cursor;
    }
    if d.mode != Mode::Object {
        d.mode = Mode::Object;
    }
    let i = d.scene.add_object(o);
    d.select_only(Some(i));
    d.report = format!("Added {name}");
}

fn lineart_data() -> GreasePencil {
    let mut g = GreasePencil::default();
    g.layers.push(GpLayer::new("Lines"));
    g.materials.push(GpMaterial::new("Black", [0.0, 0.0, 0.0, 1.0]));
    g
}

/// `--gp-demo`: the cat head in front of the cube with a drop shadow, a
/// wavy stroke beside it, Draw Mode in the 2D Animation workspace.
pub fn demo(d: &mut Doc) {
    if let Some(c) = d.scene.objects.iter().position(|o| o.name == "Cube") {
        d.scene.objects[c].transform.location = sex_3d::Vec3::new(0.6, 2.5, -0.2);
    }
    d.cursor = sex_3d::Vec3::new(2.2, 0.0, 0.0);
    add_object(d, "STROKE");
    if let Some(a) = d.active() {
        if let Some(mut m) = GpModifier::new("NOISE") {
            m.name = "Noise".into();
            d.scene.objects[a].gp_modifiers.push(m);
        }
    }
    d.cursor = sex_3d::Vec3::ZERO;
    add_object(d, "MONKEY");
    if let Some(a) = d.active() {
        if let Some(e) = GpEffect::new("FX_SHADOW") {
            d.scene.objects[a].shader_fx.push(e);
        }
    }
    crate::rig::mode_set(d, "PAINT_GREASE_PENCIL");
    d.gp.brush.radius = 4.0;
    d.want_workspace = Some("2D Animation".into());
    d.touch();
}

/// The Add menu's Grease Pencil submenu.
pub fn add_menu_items() -> Vec<MenuItem> {
    let op = |l: &str, a: &str| MenuItem::op(l, "", Op::with("object.grease_pencil_add", a));
    vec![
        op("Blank", "EMPTY"),
        op("Stroke", "STROKE"),
        op("Cat Head", "MONKEY"),
        MenuItem::Separator,
        op("Scene Line Art", "LINEART_SCENE"),
        op("Collection Line Art", "LINEART_COLLECTION"),
        op("Object Line Art", "LINEART_OBJECT"),
    ]
}

// ---------------------------------------------------------------- operators

fn report_none(d: &mut Doc, what: &str) {
    d.report = format!("{what}: no Grease Pencil object active");
}

fn select_action(arg: &str) -> SelectAction {
    match arg {
        "DESELECT" => SelectAction::Deselect,
        "INVERT" => SelectAction::Invert,
        "TOGGLE" => SelectAction::Toggle,
        _ => SelectAction::Select,
    }
}

/// Set a tool or brush setting from `key=value` (header widgets and menus).
fn set_setting(d: &mut Doc, arg: &str) {
    let Some((k, v)) = arg.split_once('=') else { return };
    let num = || v.parse::<f32>().ok();
    let st = &mut d.gp;
    match k {
        "plane" => st.plane = DrawPlane::from_id(v).unwrap_or(st.plane),
        "placement" => st.placement = Placement::from_id(v).unwrap_or(st.placement),
        "erase_mode" => st.erase_mode = EraseMode::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(v)).unwrap_or(st.erase_mode),
        "select_mode" => {
            st.select_mode = match v {
                "STROKE" => SelectMode::Stroke,
                "SEGMENT" => SelectMode::Segment,
                _ => SelectMode::Point,
            }
        }
        "easing" => st.easing = Easing::from_id(v),
        "additive" => st.additive = v == "1",
        "stabilizer" => st.brush.use_stabilizer = v == "1",
        "caps" => st.brush.caps = if v == "FLAT" { CapType::Flat } else { CapType::Round },
        "radius" => st.brush.radius = num().unwrap_or(st.brush.radius).max(1.0),
        "strength" => st.brush.strength = num().unwrap_or(st.brush.strength).clamp(0.0, 1.0),
        "vertex_mode" => {
            st.vertex_stroke = v != "FILL";
            st.vertex_fill = v != "STROKE";
        }
        _ => d.report = format!("Unknown Grease Pencil setting '{k}'"),
    }
}

/// Grease Pencil operators. Returns false for names it does not know.
pub fn run(d: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    let id = op.id.as_str();
    let gp_op = id.starts_with("grease_pencil.") || id.starts_with("gpencil.") || matches!(id, "object.grease_pencil_add" | "object.gpencil_modifier_add" | "object.gpencil_modifier_remove" | "object.gpencil_modifier_move" | "object.gpencil_modifier_apply" | "object.shaderfx_add" | "object.shaderfx_remove" | "object.shaderfx_move");
    if !gp_op {
        return false;
    }
    match id {
        "object.grease_pencil_add" => add_object(d, arg),
        "gpencil.set" => set_setting(d, arg),
        "gpencil.tool" => {
            match d.mode {
                Mode::Sculpt => d.gp.sculpt_tool = SculptTool::from_label(arg).unwrap_or(d.gp.sculpt_tool),
                Mode::VertexPaint => d.gp.vertex_tool = VertexTool::from_label(arg).unwrap_or(d.gp.vertex_tool),
                Mode::WeightPaint => d.gp.weight_tool = WeightTool::ALL.into_iter().find(|t| t.label() == arg).unwrap_or(d.gp.weight_tool),
                Mode::Edit => d.gp.edit_tool = EditTool::ALL.into_iter().find(|t| t.label() == arg).unwrap_or(d.gp.edit_tool),
                _ => d.gp.tool = DrawTool::from_label(arg).unwrap_or(d.gp.tool),
            }
            d.touch();
        }
        "gpencil.prop_set" => ui::prop_set(d, arg),
        "gpencil.dopesheet_keytype" => {
            let Some(kt) = KeyframeType::from_id(arg) else { return true };
            d.push_undo("Set Keyframe Type");
            for g in &mut d.scene.grease_pencils {
                for l in &mut g.layers {
                    for f in l.frames.iter_mut().filter(|f| f.select) {
                        f.key_type = kt;
                    }
                }
            }
            d.touch();
        }
        "gpencil.onion_toggle" => {
            if let Some((_, g)) = active_gp(d) {
                let o = &mut d.scene.grease_pencils[g].onion;
                o.enabled = !o.enabled;
                d.touch();
            }
        }
        _ => {
            let Some((oi, g)) = active_gp(d) else {
                report_none(d, id);
                return true;
            };
            run_data(d, id, arg, oi, g);
        }
    }
    true
}

fn run_data(d: &mut Doc, id: &str, arg: &str, oi: usize, g: usize) {
    let frame = d.frame;
    match id {
        // ---- layers
        "grease_pencil.layer_add" => {
            d.push_undo("Add Layer");
            let gpd = &mut d.scene.grease_pencils[g];
            let name = if arg.is_empty() { "Layer" } else { arg };
            let li = gpd.add_layer(name);
            // New layers go above the active one's group.
            gpd.layers[li].group = None;
            d.touch();
        }
        "grease_pencil.layer_remove" => {
            let gpd = &d.scene.grease_pencils[g];
            if gpd.layers.is_empty() {
                return;
            }
            d.push_undo("Remove Layer");
            let gpd = &mut d.scene.grease_pencils[g];
            let i = gpd.active_layer;
            gpd.remove_layer(i);
            d.touch();
        }
        "grease_pencil.layer_move" => {
            let gpd = &d.scene.grease_pencils[g];
            let i = gpd.active_layer;
            // The list shows the top layer first: "UP" moves towards the end.
            let j = if arg == "DOWN" { i.checked_sub(1) } else { (i + 1 < gpd.layers.len()).then_some(i + 1) };
            if let Some(j) = j {
                d.push_undo("Move Layer");
                let gpd = &mut d.scene.grease_pencils[g];
                gpd.layers.swap(i, j);
                gpd.active_layer = j;
                d.touch();
            }
        }
        "grease_pencil.layer_active" => {
            if let Ok(i) = arg.parse::<usize>() {
                let gpd = &mut d.scene.grease_pencils[g];
                if i < gpd.layers.len() {
                    gpd.active_layer = i;
                    d.touch();
                }
            }
        }
        "grease_pencil.layer_duplicate" => {
            let gpd = &d.scene.grease_pencils[g];
            let Some(l) = gpd.layers.get(gpd.active_layer).cloned() else { return };
            d.push_undo("Duplicate Layer");
            let gpd = &mut d.scene.grease_pencils[g];
            let i = gpd.add_layer(&l.name);
            let name = gpd.layers[i].name.clone();
            let mut l = l;
            l.name = name;
            if arg == "EMPTY" {
                for f in &mut l.frames {
                    f.drawing = Drawing::default();
                }
            }
            gpd.layers[i] = l;
            d.touch();
        }
        "grease_pencil.layer_hide" => {
            d.push_undo("Hide Layers");
            let gpd = &mut d.scene.grease_pencils[g];
            let act = gpd.active_layer;
            for (i, l) in gpd.layers.iter_mut().enumerate() {
                if (arg == "UNSELECTED") != (i == act) {
                    l.hide = true;
                }
            }
            d.touch();
        }
        "grease_pencil.layer_reveal" => {
            d.push_undo("Show All Layers");
            for l in &mut d.scene.grease_pencils[g].layers {
                l.hide = false;
            }
            d.touch();
        }
        "grease_pencil.layer_lock_all" => {
            d.push_undo("Lock Layers");
            let lock = arg != "UNLOCK";
            for l in &mut d.scene.grease_pencils[g].layers {
                l.lock = lock;
            }
            d.touch();
        }
        "grease_pencil.layer_merge" => {
            // Merge the active layer down into the one below it.
            let gpd = &d.scene.grease_pencils[g];
            let i = gpd.active_layer;
            if i == 0 || i >= gpd.layers.len() {
                d.report = "No layer below to merge into".into();
                return;
            }
            d.push_undo("Merge Down");
            let gpd = &mut d.scene.grease_pencils[g];
            let top = gpd.layers.remove(i);
            let below = &mut gpd.layers[i - 1];
            for f in top.frames {
                let fi = below.ensure_key(f.frame, true);
                below.frames[fi].drawing.strokes.extend(f.drawing.strokes);
            }
            gpd.active_layer = i - 1;
            d.touch();
        }
        "grease_pencil.layer_group_add" => {
            d.push_undo("Add Layer Group");
            let gpd = &mut d.scene.grease_pencils[g];
            let name = sex_3d::scene::unique_name("Group", |n| gpd.groups.iter().any(|x| x.name == n));
            gpd.groups.push(LayerGroup { name, parent: None, hide: false, lock: false, expanded: true, color_tag: 0 });
            let gi = gpd.groups.len() - 1;
            let act = gpd.active_layer;
            if let Some(l) = gpd.layers.get_mut(act) {
                l.group = Some(gi);
            }
            d.touch();
        }
        // ---- materials and vertex groups
        "grease_pencil.material_slot_add" => {
            d.push_undo("Add Material Slot");
            let gpd = &mut d.scene.grease_pencils[g];
            let n = sex_3d::scene::unique_name("Material", |n| gpd.materials.iter().any(|m| m.name == n));
            let m = if arg == "FILL" { GpMaterial::fill(&n, [0.5, 0.5, 0.5, 1.0]) } else { GpMaterial::new(&n, [0.0, 0.0, 0.0, 1.0]) };
            gpd.materials.push(m);
            gpd.active_material = gpd.materials.len() - 1;
            d.touch();
        }
        "grease_pencil.material_slot_remove" => {
            let gpd = &d.scene.grease_pencils[g];
            if gpd.materials.len() <= 1 {
                d.report = "A Grease Pencil object keeps at least one material".into();
                return;
            }
            d.push_undo("Remove Material Slot");
            let gpd = &mut d.scene.grease_pencils[g];
            let mi = gpd.active_material.min(gpd.materials.len() - 1);
            gpd.materials.remove(mi);
            for l in &mut gpd.layers {
                for f in &mut l.frames {
                    for s in &mut f.drawing.strokes {
                        if s.material > mi {
                            s.material -= 1;
                        } else if s.material == mi {
                            s.material = 0;
                        }
                    }
                }
            }
            gpd.active_material = mi.saturating_sub(1);
            d.touch();
        }
        "grease_pencil.material_active" => {
            if let Ok(i) = arg.parse::<usize>() {
                let gpd = &mut d.scene.grease_pencils[g];
                if i < gpd.materials.len() {
                    gpd.active_material = i;
                    d.touch();
                }
            }
        }
        "grease_pencil.vertex_group_add" => {
            d.push_undo("Add Vertex Group");
            let gpd = &mut d.scene.grease_pencils[g];
            let n = sex_3d::scene::unique_name("Group", |n| gpd.vertex_groups.iter().any(|x| x == n));
            gpd.vertex_groups.push(n);
            gpd.active_vertex_group = gpd.vertex_groups.len() - 1;
            d.touch();
        }
        "grease_pencil.vertex_group_remove" => {
            let gpd = &d.scene.grease_pencils[g];
            if gpd.vertex_groups.is_empty() {
                return;
            }
            d.push_undo("Remove Vertex Group");
            let gpd = &mut d.scene.grease_pencils[g];
            let vg = gpd.active_vertex_group.min(gpd.vertex_groups.len() - 1) as u32;
            gpd.vertex_groups.remove(vg as usize);
            for l in &mut gpd.layers {
                for f in &mut l.frames {
                    for s in &mut f.drawing.strokes {
                        for p in &mut s.points {
                            p.weights.retain(|w| w.0 != vg);
                            for w in &mut p.weights {
                                if w.0 > vg {
                                    w.0 -= 1;
                                }
                            }
                        }
                    }
                }
            }
            gpd.active_vertex_group = (vg as usize).saturating_sub(1);
            d.touch();
        }
        "grease_pencil.vertex_group_active" => {
            if let Ok(i) = arg.parse::<usize>() {
                d.scene.grease_pencils[g].active_vertex_group = i;
                d.touch();
            }
        }
        "grease_pencil.vertex_group_assign" | "grease_pencil.vertex_group_remove_from" => {
            let gpd = &d.scene.grease_pencils[g];
            if gpd.vertex_groups.is_empty() {
                d.report = "No vertex group".into();
                return;
            }
            let vg = gpd.active_vertex_group.min(gpd.vertex_groups.len() - 1) as u32;
            let assign = id.ends_with("assign");
            let w = d.gp.weight_value;
            d.push_undo(if assign { "Assign to Vertex Group" } else { "Remove from Vertex Group" });
            each_drawing(d, |dr, _| {
                for s in &mut dr.strokes {
                    for p in s.points.iter_mut().filter(|p| p.select) {
                        if assign {
                            p.set_weight(vg, w);
                        } else {
                            p.weights.retain(|x| x.0 != vg);
                        }
                    }
                }
                true
            });
            d.touch();
        }
        // ---- selection
        "grease_pencil.select_all" => {
            let a = select_action(arg);
            each_drawing(d, |dr, _| {
                gedit::select_all(dr, a);
                true
            });
            d.touch();
        }
        "grease_pencil.select_linked" => {
            each_drawing(d, |dr, _| {
                gedit::select_linked(dr);
                true
            });
            d.touch();
        }
        "grease_pencil.select_more" => {
            each_drawing(d, |dr, _| {
                gedit::select_more(dr);
                true
            });
            d.touch();
        }
        "grease_pencil.select_alternate" => {
            each_drawing(d, |dr, _| {
                for s in &mut dr.strokes {
                    if s.points.iter().any(|p| p.select) {
                        for (i, p) in s.points.iter_mut().enumerate() {
                            p.select = i % 2 == 0;
                        }
                    }
                }
                gedit::sync_stroke_select(dr);
                true
            });
            d.touch();
        }
        // ---- stroke edits
        "grease_pencil.delete" | "grease_pencil.dissolve" => {
            let label = if id.ends_with("dissolve") { "Dissolve" } else { "Delete" };
            d.push_undo(label);
            let n = each_drawing(d, |dr, _| match (id, arg) {
                ("grease_pencil.dissolve", _) => gedit::dissolve_points(dr),
                (_, "STROKES") => gedit::delete_strokes(dr),
                _ => gedit::delete_points(dr),
            });
            if !n {
                d.report = "Nothing selected".into();
            }
            d.touch();
        }
        "grease_pencil.delete_frame" => {
            d.push_undo("Delete Frame");
            let gpd = &mut d.scene.grease_pencils[g];
            let all = arg == "ALL_FRAMES";
            let act = gpd.active_layer;
            for (i, l) in gpd.layers.iter_mut().enumerate() {
                if all || i == act {
                    if let Some(fi) = l.frame_index_at(frame) {
                        l.frames.remove(fi);
                    }
                }
            }
            d.touch();
        }
        "grease_pencil.insert_blank_frame" => {
            d.push_undo("Insert Blank Frame");
            let gpd = &mut d.scene.grease_pencils[g];
            let all = arg == "ALL_LAYERS";
            let act = gpd.active_layer;
            for (i, l) in gpd.layers.iter_mut().enumerate() {
                if all || i == act {
                    let fi = l.ensure_key(frame, false);
                    l.frames[fi].drawing = Drawing::default();
                }
            }
            d.touch();
        }
        "grease_pencil.frame_duplicate" => {
            d.push_undo("Duplicate Frame");
            let gpd = &mut d.scene.grease_pencils[g];
            let act = gpd.active_layer;
            if let Some(l) = gpd.layers.get_mut(act) {
                if let Some(fi) = l.frame_index_at(frame) {
                    let mut f = l.frames[fi].clone();
                    f.frame = frame + 1;
                    if let Some(j) = l.key_at(frame + 1) {
                        l.frames[j] = f;
                    } else {
                        l.frames.push(f);
                        l.sort_frames();
                    }
                }
            }
            d.touch();
        }
        "grease_pencil.keyframe_type" => {
            let Some(kt) = KeyframeType::from_id(arg) else { return };
            d.push_undo("Set Keyframe Type");
            for l in &mut d.scene.grease_pencils[g].layers {
                for f in l.frames.iter_mut().filter(|f| f.select) {
                    f.key_type = kt;
                }
            }
            d.touch();
        }
        "grease_pencil.duplicate" => {
            d.push_undo("Duplicate");
            each_drawing(d, |dr, _| gedit::duplicate(dr) > 0);
            d.touch();
        }
        "grease_pencil.stroke_join" => {
            d.push_undo("Join Strokes");
            let copy = arg == "JOINCOPY";
            if !each_drawing(d, |dr, _| gedit::join(dr, copy)) {
                d.report = "Select at least two strokes to join".into();
            }
            d.touch();
        }
        "grease_pencil.stroke_subdivide" => {
            let cuts = arg.parse().unwrap_or(1u32).max(1);
            d.push_undo("Subdivide Stroke");
            each_drawing(d, |dr, _| gedit::subdivide(dr, cuts));
            d.touch();
        }
        "grease_pencil.stroke_simplify" => {
            let f = arg.parse().unwrap_or(0.01f32);
            d.push_undo("Simplify Stroke");
            each_drawing(d, |dr, _| gedit::simplify(dr, f));
            d.touch();
        }
        "grease_pencil.stroke_smooth" => {
            let iters = arg.parse().unwrap_or(2u32).max(1);
            d.push_undo("Smooth Stroke");
            each_drawing(d, |dr, _| {
                gedit::smooth(dr, 0.5, iters, SmoothWhat::POSITION);
                true
            });
            d.touch();
        }
        "grease_pencil.caps_set" => {
            d.push_undo("Set Caps");
            let (s, e) = match arg {
                "ROUND" => (Some(CapType::Round), Some(CapType::Round)),
                "FLAT" => (Some(CapType::Flat), Some(CapType::Flat)),
                "START" => (None, None),
                _ => (None, None),
            };
            each_drawing(d, |dr, _| {
                if s.is_none() {
                    // Toggle (both ends, or the start only).
                    for st in dr.strokes.iter_mut().filter(|st| st.select) {
                        let flip = |c: CapType| if c == CapType::Round { CapType::Flat } else { CapType::Round };
                        st.start_cap = flip(st.start_cap);
                        if arg != "START" {
                            st.end_cap = flip(st.end_cap);
                        }
                    }
                } else {
                    gedit::set_caps(dr, s, e);
                }
                true
            });
            d.touch();
        }
        "grease_pencil.cyclical_set" => {
            d.push_undo("Toggle Cyclic");
            each_drawing(d, |dr, _| gedit::toggle_cyclic(dr));
            d.touch();
        }
        "grease_pencil.stroke_switch_direction" => {
            d.push_undo("Switch Direction");
            each_drawing(d, |dr, _| {
                gedit::reverse(dr);
                true
            });
            d.touch();
        }
        "grease_pencil.stroke_material_set" => {
            let m = arg.parse().unwrap_or(d.scene.grease_pencils[g].active_material);
            d.push_undo("Assign Material");
            each_drawing(d, |dr, _| gedit::set_material(dr, m));
            d.touch();
        }
        "grease_pencil.move_to_layer" => {
            let gpd = &d.scene.grease_pencils[g];
            let target = match gpd.find_layer(arg) {
                Some(t) => t,
                None if !arg.is_empty() => usize::MAX,
                None => return,
            };
            d.push_undo("Move to Layer");
            let target = if target == usize::MAX { d.scene.grease_pencils[g].add_layer(arg) } else { target };
            let mut moved = Vec::new();
            let m = editable(d, oi, g);
            for (li, fi, _) in m {
                if li != target {
                    moved.extend(gedit::extract_selected(&mut d.scene.grease_pencils[g].layers[li].frames[fi].drawing));
                }
            }
            let l = &mut d.scene.grease_pencils[g].layers[target];
            let fi = l.ensure_key(frame, false);
            l.frames[fi].drawing.strokes.extend(moved);
            d.touch();
        }
        "grease_pencil.copy" => {
            let mut clip = Vec::new();
            let gpd = &d.scene.grease_pencils[g];
            for (li, fi, _) in editable(d, oi, g) {
                clip.extend(gpd.layers[li].frames[fi].drawing.strokes.iter().filter(|s| s.select).cloned());
            }
            d.report = format!("Copied {} strokes", clip.len());
            d.gp.clipboard = clip;
        }
        "grease_pencil.paste" => {
            if d.gp.clipboard.is_empty() {
                d.report = "Nothing to paste".into();
                return;
            }
            d.push_undo("Paste Strokes");
            let clip = d.gp.clipboard.clone();
            let gpd = &mut d.scene.grease_pencils[g];
            let act = gpd.active_layer;
            if let Some(l) = gpd.layers.get_mut(act) {
                let fi = l.ensure_key(frame, true);
                let dr = &mut l.frames[fi].drawing;
                gedit::select_all(dr, SelectAction::Deselect);
                dr.strokes.extend(clip);
            }
            d.touch();
        }
        "grease_pencil.separate" => separate(d, oi, g),
        "grease_pencil.stroke_reset_vertex_color" => {
            d.push_undo("Reset Vertex Color");
            each_drawing(d, |dr, _| {
                for s in &mut dr.strokes {
                    for p in &mut s.points {
                        p.vertex_color = [0.0; 4];
                    }
                    s.fill_color = [0.0; 4];
                }
                true
            });
            d.touch();
        }
        // ---- interpolation
        "grease_pencil.interpolate" | "grease_pencil.interpolate_sequence" => {
            let easing = if arg.is_empty() { d.gp.easing } else { Easing::from_id(arg) };
            let seq = id.ends_with("sequence");
            let step = d.gp.interp_step.max(1);
            d.push_undo(if seq { "Interpolate Sequence" } else { "Interpolate" });
            let gpd = &mut d.scene.grease_pencils[g];
            let act = gpd.active_layer;
            let mut n = 0;
            for (i, l) in gpd.layers.iter_mut().enumerate() {
                if i != act && !seq {
                    continue;
                }
                if seq {
                    n += interp::interpolate_sequence(l, frame, step, easing);
                } else if interp::interpolate(l, frame, easing) {
                    n += 1;
                }
            }
            d.report = if n == 0 { "Interpolation needs keyframes before and after the current frame".into() } else { format!("Interpolated {n} frame(s)") };
            d.touch();
        }
        // ---- modifiers and effects
        "object.gpencil_modifier_add" => {
            let Some(m) = GpModifier::new(arg) else {
                d.report = format!("Unknown Grease Pencil modifier '{arg}'");
                return;
            };
            d.push_undo(&format!("Add {} Modifier", m.label()));
            let ob = &mut d.scene.objects[oi];
            let mut m = m;
            m.name = sex_3d::scene::unique_name(&m.name, |n| ob.gp_modifiers.iter().any(|x| x.name == n));
            ob.gp_modifiers.push(m);
            d.touch();
        }
        "object.gpencil_modifier_remove" | "object.shaderfx_remove" => {
            let Ok(i) = arg.parse::<usize>() else { return };
            d.push_undo("Remove");
            let ob = &mut d.scene.objects[oi];
            if id.contains("shaderfx") {
                if i < ob.shader_fx.len() {
                    ob.shader_fx.remove(i);
                }
            } else if i < ob.gp_modifiers.len() {
                ob.gp_modifiers.remove(i);
            }
            d.touch();
        }
        "object.gpencil_modifier_move" | "object.shaderfx_move" => {
            let Some((i, dir)) = arg.split_once(':') else { return };
            let Ok(i) = i.parse::<usize>() else { return };
            d.push_undo("Move");
            let ob = &mut d.scene.objects[oi];
            let len = if id.contains("shaderfx") { ob.shader_fx.len() } else { ob.gp_modifiers.len() };
            let j = if dir == "UP" { i.checked_sub(1) } else { (i + 1 < len).then_some(i + 1) };
            if let Some(j) = j {
                if id.contains("shaderfx") {
                    ob.shader_fx.swap(i, j);
                } else {
                    ob.gp_modifiers.swap(i, j);
                }
            }
            d.touch();
        }
        "object.gpencil_modifier_apply" => {
            let Ok(i) = arg.parse::<usize>() else { return };
            if i >= d.scene.objects[oi].gp_modifiers.len() {
                return;
            }
            d.push_undo("Apply Modifier");
            apply_modifier(d, oi, g, i);
            d.touch();
        }
        "object.shaderfx_add" => {
            let Some(e) = GpEffect::new(arg) else {
                d.report = format!("Unknown visual effect '{arg}'");
                return;
            };
            d.push_undo(&format!("Add {} Effect", e.label()));
            d.scene.objects[oi].shader_fx.push(e);
            d.touch();
        }
        _ => d.report = format!("Unknown operator {id}"),
    }
}

/// Bake modifier `i` into the data (every keyframe of every layer).
pub fn apply_modifier(d: &mut Doc, oi: usize, g: usize, i: usize) {
    let m = d.scene.objects[oi].gp_modifiers[i].clone();
    let gpd = d.scene.grease_pencils[g].clone();
    let mut out = gpd.clone();
    let mut frames: Vec<i32> = gpd.key_frames();
    if frames.is_empty() {
        frames.push(d.frame);
    }
    for &f in &frames {
        let ev = evaluate(&gpd, std::slice::from_ref(&m), &EvalContext { scene: Some(&d.scene), owner: oi, frame: f, render: false });
        for el in ev {
            let Some(l) = out.layers.get_mut(el.layer) else { continue };
            // Generated layers (Line Art) get a key on this frame.
            let fi = match el.key_frame {
                Some(k) => l.key_at(k).unwrap_or_else(|| l.ensure_key(k, false)),
                None => l.ensure_key(f, false),
            };
            l.frames[fi].drawing.strokes = el.strokes;
        }
    }
    d.scene.grease_pencils[g] = out;
    d.scene.objects[oi].gp_modifiers.remove(i);
}

/// P: move the selected strokes to a new object.
fn separate(d: &mut Doc, oi: usize, g: usize) {
    let lay = editable(d, oi, g);
    let any = lay.iter().any(|&(li, fi, _)| d.scene.grease_pencils[g].layers[li].frames[fi].drawing.strokes.iter().any(|s| s.select));
    if !any {
        d.report = "Nothing selected".into();
        return;
    }
    d.push_undo("Separate");
    let src = d.scene.grease_pencils[g].clone();
    let mut new = GreasePencil { layers: Vec::new(), ..src.clone() };
    for (li, l) in src.layers.iter().enumerate() {
        let mut nl = GpLayer { frames: Vec::new(), ..l.clone() };
        if let Some(&(_, fi, _)) = lay.iter().find(|x| x.0 == li) {
            let moved = gedit::extract_selected(&mut d.scene.grease_pencils[g].layers[li].frames[fi].drawing);
            let mut f = l.frames[fi].clone();
            f.drawing.strokes = moved;
            nl.frames.push(f);
        }
        new.layers.push(nl);
    }
    let name = format!("{}_separated", d.scene.objects[oi].name);
    let ng = d.scene.add_grease_pencil(&name, new);
    let mut o = d.scene.objects[oi].clone();
    o.name = name;
    o.data = ObjectData::GreasePencil(ng);
    d.scene.add_object(o);
    d.touch();
}

/// Strokes the Dope Sheet's Grease Pencil mode shows: (object, layer
/// name, keyframes as (frame, type, selected)).
pub fn dopesheet_rows(d: &Doc) -> Vec<(usize, usize, String, Vec<(i32, KeyframeType, bool)>)> {
    let mut v = Vec::new();
    for (oi, o) in d.scene.objects.iter().enumerate() {
        let ObjectData::GreasePencil(g) = o.data else { continue };
        let Some(gpd) = d.scene.grease_pencils.get(g) else { continue };
        for (li, l) in gpd.layers.iter().enumerate().rev() {
            v.push((oi, li, l.name.clone(), l.frames.iter().map(|f| (f.frame, f.key_type, f.select)).collect()));
        }
    }
    v
}

/// Move the selected Grease Pencil keyframes by `df` frames (Dope Sheet G).
pub fn move_keys(d: &mut Doc, df: i32) {
    for gpd in &mut d.scene.grease_pencils {
        for l in &mut gpd.layers {
            for f in l.frames.iter_mut().filter(|f| f.select) {
                f.frame += df;
            }
            l.frames.sort_by_key(|f| (f.frame, !f.select));
            l.frames.dedup_by_key(|f| f.frame);
        }
    }
    d.touch();
}

/// The modifier ids and labels for the Add Modifier menu, grouped like
/// Blender's (Modify, Generate, Deform, Color).
pub fn modifier_menu() -> Vec<MenuItem> {
    let item = |id: &str| {
        let label = modifiers::TYPES.iter().find(|t| t.0 == id).map_or(id, |t| t.1);
        MenuItem::op(label, "", Op::with("object.gpencil_modifier_add", id))
    };
    let group = |name: &str, ids: &[&str]| MenuItem::Sub(name.into(), ids.iter().map(|i| item(i)).collect());
    vec![
        group("Edit", &["TEXTURE", "TIME", "OPACITY", "TINT"]),
        group("Generate", &["ARRAY", "BUILD", "DASH", "ENVELOPE", "LENGTH", "LINEART", "MIRROR", "MULTIPLY", "OUTLINE", "SIMPLIFY", "SUBDIV"]),
        group("Deform", &["ARMATURE", "HOOK", "LATTICE", "NOISE", "OFFSET", "SMOOTH", "THICKNESS"]),
    ]
}

pub fn effect_menu() -> Vec<MenuItem> {
    sex_3d::grease_pencil::effects::TYPES.iter().map(|(id, label)| MenuItem::op(label, "", Op::with("object.shaderfx_add", id))).collect()
}
