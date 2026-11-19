// SPDX-License-Identifier: GPL-3.0-only
//! Properties panels drawn from the settings visitors of
//! `sex_3d::props`: a Grease Pencil layer, material, modifier or visual
//! effect, or a sequencer strip, lists its fields once and this module
//! draws them. Edits are recorded as `key=value` pairs and applied after the
//! undo step is pushed, so each change is one undo step.
use crate::doc::Doc;
use crate::draw::Rect;
use crate::editors::properties::layout::L;
use crate::editors::properties::ColorTarget;
use crate::ui::{id, MenuItem, NumOpts, Op};
use sex_3d::grease_pencil::visit::visit_transform;
use sex_3d::grease_pencil::GpLayer;
use sex_3d::props::{apply_pairs, fmt_num, NameKind, Range, Visit};
use sex_3d::scene::ObjectData;

/// Something whose settings a panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpOwner {
    /// Grease Pencil data-block settings (onion skin, depth order).
    Data(usize),
    Layer(usize, usize),
    Group(usize, usize),
    Material(usize, usize),
    /// Object, modifier index.
    Modifier(usize, usize),
    Effect(usize, usize),
    /// Sequencer strip: timing, look (blend, transform, colour) and kind.
    StripTime(usize),
    StripLook(usize),
    StripKind(usize),
    StripMod(usize, usize),
    Channel(u32),
    /// Grease Pencil tool settings (brush colours).
    Tool,
}

impl GpOwner {
    pub fn to_id(self) -> String {
        match self {
            GpOwner::Data(g) => format!("data:{g}"),
            GpOwner::Layer(g, i) => format!("layer:{g}:{i}"),
            GpOwner::Group(g, i) => format!("group:{g}:{i}"),
            GpOwner::Material(g, i) => format!("material:{g}:{i}"),
            GpOwner::Modifier(o, i) => format!("modifier:{o}:{i}"),
            GpOwner::Effect(o, i) => format!("effect:{o}:{i}"),
            GpOwner::StripTime(s) => format!("strip_time:{s}"),
            GpOwner::StripLook(s) => format!("strip_look:{s}"),
            GpOwner::StripKind(s) => format!("strip_kind:{s}"),
            GpOwner::StripMod(s, i) => format!("strip_mod:{s}:{i}"),
            GpOwner::Channel(c) => format!("channel:{c}"),
            GpOwner::Tool => "tool".into(),
        }
    }
    pub fn from_id(s: &str) -> Option<GpOwner> {
        let mut it = s.split(':');
        let kind = it.next()?;
        let mut n = || it.next().and_then(|x| x.parse::<usize>().ok());
        Some(match kind {
            "data" => GpOwner::Data(n()?),
            "layer" => GpOwner::Layer(n()?, n()?),
            "group" => GpOwner::Group(n()?, n()?),
            "material" => GpOwner::Material(n()?, n()?),
            "modifier" => GpOwner::Modifier(n()?, n()?),
            "effect" => GpOwner::Effect(n()?, n()?),
            "strip_time" => GpOwner::StripTime(n()?),
            "strip_look" => GpOwner::StripLook(n()?),
            "strip_kind" => GpOwner::StripKind(n()?),
            "strip_mod" => GpOwner::StripMod(n()?, n()?),
            "channel" => GpOwner::Channel(n()? as u32),
            "tool" => GpOwner::Tool,
            _ => return None,
        })
    }
    /// The Grease Pencil data-block the owner belongs to.
    fn gp(self, d: &Doc) -> Option<usize> {
        match self {
            GpOwner::Data(g) | GpOwner::Layer(g, _) | GpOwner::Group(g, _) | GpOwner::Material(g, _) => Some(g),
            GpOwner::Modifier(o, _) | GpOwner::Effect(o, _) => match d.scene.objects.get(o)?.data {
                ObjectData::GreasePencil(g) => Some(g),
                _ => None,
            },
            _ => None,
        }
    }
    /// Is this a tool setting rather than scene data?
    fn is_tool(self) -> bool {
        self == GpOwner::Tool
    }
}

/// Layer settings beyond [`GpLayer::visit`]: tint, radius offset, masks,
/// onion skinning, pass index, parent and transform.
fn layer_all(l: &mut GpLayer, v: &mut dyn Visit) {
    l.visit(v);
    v.heading("Adjustments");
    v.floats("tint_color", "Tint Color", &mut l.tint_color, Range::FACTOR, true);
    v.float("tint_factor", "Tint Factor", &mut l.tint_factor, Range::FACTOR);
    v.float("radius_offset", "Stroke Thickness", &mut l.radius_offset, Range::new(-1.0, 1.0));
    v.heading("Relations");
    let mut parent = l.parent.clone().unwrap_or_default();
    v.name("parent", "Parent", &mut parent, NameKind::Object);
    l.parent = (!parent.is_empty()).then_some(parent);
    v.uint("pass_index", "Pass Index", &mut l.pass_index, 0, 255);
    v.boolean("use_onion_skinning", "Onion Skinning", &mut l.use_onion_skinning);
    v.boolean("use_masks", "Use Masks", &mut l.use_masks);
    v.heading("Transform");
    visit_transform(v, &mut l.transform);
}

fn tool_visit(st: &mut super::GpState, v: &mut dyn Visit) {
    v.floats("brush_color", "Color", &mut st.brush.vertex_color, Range::FACTOR, true);
    v.floats("tint_color", "Tint Color", &mut st.tint_color, Range::FACTOR, true);
    v.floats("vertex_color", "Vertex Color", &mut st.vertex_color, Range::FACTOR, true);
    v.float("weight", "Weight", &mut st.weight_value, Range::FACTOR);
}

/// Walk the owner's settings with `v`. False if the owner does not exist.
pub fn with(d: &mut Doc, o: GpOwner, v: &mut dyn Visit) -> bool {
    if o.is_tool() {
        tool_visit(&mut d.gp, v);
        return true;
    }
    let s = &mut d.scene;
    match o {
        GpOwner::Data(g) => s.grease_pencils.get_mut(g).map(|x| x.visit(v)).is_some(),
        GpOwner::Layer(g, i) => s.grease_pencils.get_mut(g).and_then(|x| x.layers.get_mut(i)).map(|x| layer_all(x, v)).is_some(),
        GpOwner::Group(g, i) => s.grease_pencils.get_mut(g).and_then(|x| x.groups.get_mut(i)).map(|x| x.visit(v)).is_some(),
        GpOwner::Material(g, i) => s.grease_pencils.get_mut(g).and_then(|x| x.materials.get_mut(i)).map(|x| x.visit(v)).is_some(),
        GpOwner::Modifier(ob, i) => s.objects.get_mut(ob).and_then(|x| x.gp_modifiers.get_mut(i)).map(|x| x.visit(v)).is_some(),
        GpOwner::Effect(ob, i) => s.objects.get_mut(ob).and_then(|x| x.shader_fx.get_mut(i)).map(|x| x.visit(v)).is_some(),
        GpOwner::StripTime(i) | GpOwner::StripLook(i) | GpOwner::StripKind(i) | GpOwner::StripMod(i, _) => {
            let Some(q) = s.sequence_editor.as_mut() else { return false };
            let Some(st) = q.strips.get_mut(i) else { return false };
            match o {
                GpOwner::StripTime(_) => st.visit_time(v),
                GpOwner::StripLook(_) => st.visit_look(v),
                GpOwner::StripKind(_) => st.visit_kind(v),
                GpOwner::StripMod(_, m) => match st.modifiers.get_mut(m) {
                    Some(m) => m.visit(v),
                    None => return false,
                },
                _ => {}
            }
            q.sync_effects();
            true
        }
        GpOwner::Channel(c) => match s.sequence_editor.as_mut() {
            Some(q) => {
                q.channel_mut(c).visit(v);
                true
            }
            None => false,
        },
        GpOwner::Tool => true,
    }
}

/// `gpencil.prop_set`: "owner<TAB>key<TAB>value" (menus of name and choice
/// fields).
pub fn prop_set(d: &mut Doc, arg: &str) {
    let mut it = arg.splitn(3, '\t');
    let (Some(o), Some(k), Some(val)) = (it.next(), it.next(), it.next()) else { return };
    let Some(o) = GpOwner::from_id(o) else { return };
    set_pairs(d, o, vec![(k.to_string(), val.to_string())], true);
}

/// Apply `pairs` to the owner, with an undo step when `undo`.
pub fn set_pairs(d: &mut Doc, o: GpOwner, pairs: Vec<(String, String)>, undo: bool) {
    if undo && !o.is_tool() {
        d.push_undo("Change Property");
    }
    let r = apply_pairs(pairs, |v| {
        with(d, o, v);
    });
    if let Err(e) = r {
        d.report = e;
    }
    d.touch();
}

/// A colour field a picker edits: the owner and the hash of its key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropColor {
    pub owner: GpOwner,
    pub key: u64,
}

impl PropColor {
    pub fn new(owner: GpOwner, key: &str) -> PropColor {
        PropColor { owner, key: id(key) }
    }
}

struct ColorAccess {
    key: u64,
    get: Option<[f32; 3]>,
    set: Option<[f32; 3]>,
}

impl Visit for ColorAccess {
    fn float(&mut self, _: &str, _: &str, _: &mut f32, _: Range) {}
    fn int(&mut self, _: &str, _: &str, _: &mut i32, _: i32, _: i32) {}
    fn boolean(&mut self, _: &str, _: &str, _: &mut bool) {}
    fn floats(&mut self, key: &str, _: &str, v: &mut [f32], _: Range, color: bool) {
        if color && v.len() >= 3 && id(key) == self.key {
            self.get = Some([v[0], v[1], v[2]]);
            if let Some(c) = self.set {
                v[..3].copy_from_slice(&c);
            }
        }
    }
    fn name(&mut self, _: &str, _: &str, _: &mut String, _: NameKind) {}
    fn choice(&mut self, _: &str, _: &str, _: &[(&str, &str)], _: &mut usize) {}
}

pub fn get_color(d: &Doc, c: PropColor) -> Option<[f32; 3]> {
    // The visitors take `&mut`: walk a copy of the one owner.
    let mut a = ColorAccess { key: c.key, get: None, set: None };
    let s = &d.scene;
    let gp = |g: usize| s.grease_pencils.get(g);
    let strip = |i: usize| s.sequence_editor.as_ref().and_then(|q| q.strips.get(i)).cloned();
    match c.owner {
        GpOwner::Tool => {
            let (mut t, mut v, mut w, mut b) = (d.gp.tint_color, d.gp.vertex_color, d.gp.weight_value, d.gp.brush.vertex_color);
            a.floats("brush_color", "", &mut b, Range::FACTOR, true);
            a.floats("tint_color", "", &mut t, Range::FACTOR, true);
            a.floats("vertex_color", "", &mut v, Range::FACTOR, true);
            a.float("weight", "", &mut w, Range::FACTOR);
        }
        GpOwner::Data(g) => gp(g)?.clone().visit(&mut a),
        GpOwner::Layer(g, i) => layer_all(&mut gp(g)?.layers.get(i)?.clone(), &mut a),
        GpOwner::Group(g, i) => gp(g)?.groups.get(i)?.clone().visit(&mut a),
        GpOwner::Material(g, i) => gp(g)?.materials.get(i)?.clone().visit(&mut a),
        GpOwner::Modifier(o, i) => s.objects.get(o)?.gp_modifiers.get(i)?.clone().visit(&mut a),
        GpOwner::Effect(o, i) => s.objects.get(o)?.shader_fx.get(i)?.clone().visit(&mut a),
        GpOwner::StripTime(i) => strip(i)?.visit_time(&mut a),
        GpOwner::StripLook(i) => strip(i)?.visit_look(&mut a),
        GpOwner::StripKind(i) => strip(i)?.visit_kind(&mut a),
        GpOwner::StripMod(i, m) => strip(i)?.modifiers.get_mut(m)?.visit(&mut a),
        GpOwner::Channel(ch) => s.sequence_editor.as_ref()?.channel(ch).visit(&mut a),
    }
    a.get
}

pub fn set_color(d: &mut Doc, c: PropColor, rgb: [f32; 3]) {
    let mut a = ColorAccess { key: c.key, get: None, set: Some(rgb) };
    with(d, c.owner, &mut a);
    if !c.owner.is_tool() {
        d.touch();
    }
}

/// Names a name field offers.
pub struct Lists {
    layers: Vec<String>,
    materials: Vec<String>,
    vgroups: Vec<String>,
    objects: Vec<String>,
    collections: Vec<String>,
    images: Vec<String>,
}

impl Lists {
    pub fn new(d: &Doc, o: GpOwner) -> Lists {
        let g = o.gp(d).and_then(|g| d.scene.grease_pencils.get(g));
        Lists {
            layers: g.map(|g| g.layers.iter().map(|l| l.name.clone()).collect()).unwrap_or_default(),
            materials: g.map(|g| g.materials.iter().map(|m| m.name.clone()).collect()).unwrap_or_default(),
            vgroups: g.map(|g| g.vertex_groups.clone()).unwrap_or_default(),
            objects: d.scene.objects.iter().map(|o| o.name.clone()).collect(),
            collections: d.scene.collections.iter().map(|c| c.name.clone()).collect(),
            images: d.scene.images.iter().map(|i| i.name.clone()).collect(),
        }
    }
    fn of(&self, k: NameKind) -> Option<&[String]> {
        Some(match k {
            NameKind::Layer => &self.layers,
            NameKind::Material => &self.materials,
            NameKind::VertexGroup => &self.vgroups,
            NameKind::Object => &self.objects,
            NameKind::Collection => &self.collections,
            NameKind::Image => &self.images,
            NameKind::Text | NameKind::File => return None,
        })
    }
}

/// Draws fields into a properties layout and records what changed.
pub struct UiVisit<'x, 'a, 'b> {
    pub l: &'x mut L<'a, 'b>,
    pub owner: GpOwner,
    pub lists: Lists,
    /// Changed fields as `key=value` pairs.
    pub changes: Vec<(String, String)>,
    pub picker: Option<(ColorTarget, Rect)>,
    /// Keys to leave out (shown elsewhere, e.g. the modifier's header toggles).
    pub skip: &'static [&'static str],
}

impl UiVisit<'_, '_, '_> {
    fn k(&self, key: &str) -> String {
        format!("{}/{key}", self.owner.to_id())
    }
    fn set_op(&self, key: &str, value: &str) -> Op {
        Op::with("gpencil.prop_set", &format!("{}\t{key}\t{value}", self.owner.to_id()))
    }
    fn skipped(&self, key: &str) -> bool {
        self.skip.contains(&key)
    }
}

fn opts(r: Range) -> NumOpts<'static> {
    if r.angle {
        return NumOpts { step: 1.0, unit: "\u{b0}", min: f32::NEG_INFINITY, max: f32::INFINITY, slider: false, int: false };
    }
    NumOpts { step: r.step, unit: "", min: r.min, max: r.max, slider: r.slider, int: false }
}

fn join(v: &[f32]) -> String {
    v.iter().map(|x| fmt_num(*x)).collect::<Vec<_>>().join(",")
}

impl Visit for UiVisit<'_, '_, '_> {
    fn float(&mut self, key: &str, label: &str, v: &mut f32, r: Range) {
        if self.skipped(key) {
            return;
        }
        let k = self.k(key);
        let mut x = if r.angle { v.to_degrees() } else { *v };
        if self.l.number(label, &k, &mut x, opts(r)) {
            let x = if r.angle { x.to_radians() } else { x };
            self.changes.push((key.into(), fmt_num(x)));
        }
    }
    fn int(&mut self, key: &str, label: &str, v: &mut i32, min: i32, max: i32) {
        if self.skipped(key) {
            return;
        }
        let k = self.k(key);
        let mut x = *v as i64;
        if self.l.int(label, &k, &mut x, min as i64, max as i64) {
            self.changes.push((key.into(), x.to_string()));
        }
    }
    fn boolean(&mut self, key: &str, label: &str, v: &mut bool) {
        if self.skipped(key) {
            return;
        }
        let mut x = *v;
        if self.l.check("", label, &mut x) {
            self.changes.push((key.into(), (x as u8).to_string()));
        }
    }
    fn floats(&mut self, key: &str, label: &str, v: &mut [f32], r: Range, color: bool) {
        if self.skipped(key) {
            return;
        }
        if color && v.len() >= 3 {
            if let Some(rect) = self.l.color(label, [v[0], v[1], v[2]]) {
                self.picker = Some((ColorTarget::Prop(PropColor::new(self.owner, key)), rect));
            }
            if v.len() == 4 {
                let k = self.k(&format!("{key}.a"));
                let mut a = v[3];
                if self.l.slider("Alpha", &k, &mut a, 0.0, 1.0) {
                    let mut all = v.to_vec();
                    all[3] = a;
                    self.changes.push((key.into(), join(&all)));
                }
            }
            return;
        }
        const AXES: [&str; 4] = ["X", "Y", "Z", "W"];
        let mut all = v.to_vec();
        let mut changed = false;
        for (i, c) in all.iter_mut().enumerate() {
            let axis = AXES[i.min(3)];
            let lab = if i == 0 { format!("{label} {axis}") } else { axis.to_string() };
            let k = self.k(&format!("{key}.{i}"));
            let mut x = if r.angle { c.to_degrees() } else { *c };
            if self.l.number(&lab, &k, &mut x, opts(r)) {
                *c = if r.angle { x.to_radians() } else { x };
                changed = true;
            }
        }
        if changed {
            self.changes.push((key.into(), join(&all)));
        }
    }
    fn name(&mut self, key: &str, label: &str, v: &mut String, kind: NameKind) {
        if self.skipped(key) {
            return;
        }
        let k = self.k(key);
        match self.lists.of(kind) {
            None => {
                let mut x = v.clone();
                if self.l.text(label, &k, &mut x) {
                    self.changes.push((key.into(), x));
                }
            }
            Some(names) => {
                let mut items = vec![MenuItem::op("(none)", "", self.set_op(key, ""))];
                items.extend(names.iter().map(|n| MenuItem::op(n, "", self.set_op(key, n))));
                self.l.dropdown_row(label, &k, v.as_str(), &|| items.clone());
            }
        }
    }
    fn choice(&mut self, key: &str, label: &str, items: &[(&str, &str)], v: &mut usize) {
        if self.skipped(key) {
            return;
        }
        let labels: Vec<&str> = items.iter().map(|x| x.1).collect();
        if items.len() <= 3 && labels.iter().map(|s| s.len()).sum::<usize>() < 24 {
            if let Some(i) = self.l.expand(label, &labels, *v) {
                self.changes.push((key.into(), items[i].0.to_string()));
            }
            return;
        }
        let k = self.k(key);
        let menu: Vec<MenuItem> = items.iter().map(|(id, lab)| MenuItem::op(lab, "", self.set_op(key, id))).collect();
        let cur = items.get(*v).map_or("", |x| x.1);
        self.l.dropdown_row(label, &k, cur, &|| menu.clone());
    }
    fn heading(&mut self, label: &str) {
        self.l.space(4);
        self.l.note(label);
    }
}

/// Draw the owner's settings; edits become one undo step each. Returns a
/// colour picker to open.
pub fn draw(l: &mut L, d: &mut Doc, o: GpOwner, skip: &'static [&'static str]) -> Option<(ColorTarget, Rect)> {
    let lists = Lists::new(d, o);
    let mut v = UiVisit { l, owner: o, lists, changes: Vec::new(), picker: None, skip };
    with(d, o, &mut v);
    let UiVisit { l, changes, picker, .. } = v;
    if !changes.is_empty() {
        if !o.is_tool() {
            l.commit(d, "Change Property");
        }
        set_pairs(d, o, changes, false);
    }
    picker
}
