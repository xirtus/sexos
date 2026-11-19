// SPDX-License-Identifier: GPL-3.0-only
//! Node trees in the Properties editor, as Blender draws them: the Material
//! and World tabs' Surface, Volume and Displacement panels list the inputs
//! of the node linked into the output, and a linked input expands into the
//! inputs of the node feeding it. The node editor's sidebar uses the same
//! rows for the active node.
use super::layout::L;
use super::ColorTarget;
use crate::doc::Doc;
use crate::draw::Rect;
use crate::nodetree::{self, NodeColor, TreeRef};
use crate::ui::{MenuItem, NumOpts, Op};
use sex_3d::nodes::{enums, registry, update, Category, Prop, SocketType, Subtype, Value};

pub type Picker = Option<(ColorTarget, Rect)>;

fn write_input(l: &mut L, d: &mut Doc, r: TreeRef, n: usize, i: usize, label: &str, v: Value) {
    l.commit(d, label);
    if let Some(s) = r.get_mut(&mut d.scene).and_then(|t| t.nodes.get_mut(n)).and_then(|n| n.inputs.get_mut(i)) {
        s.value = v;
    }
    nodetree::after_edit(d, r);
}

fn write_prop(l: &mut L, d: &mut Doc, r: TreeRef, n: usize, k: usize, label: &str, v: Prop) {
    l.commit(d, label);
    if let Some(node) = r.get_mut(&mut d.scene).and_then(|t| t.nodes.get_mut(n)) {
        if let Some(p) = node.props.get_mut(k) {
            p.1 = v;
        }
        update::update(node);
    }
    nodetree::after_edit(d, r);
}

/// Items for an Id property's menu (images, objects or node groups).
fn id_items(d: &Doc, r: TreeRef, n: usize, prop: &str) -> Vec<MenuItem> {
    let names: Vec<String> = match prop {
        "image" => d.scene.images.iter().map(|i| i.name.clone()).collect(),
        "object" => d.scene.objects.iter().map(|o| o.name.clone()).collect(),
        "node_tree" => d.scene.node_groups.iter().map(|g| g.name.clone()).collect(),
        _ => Vec::new(),
    };
    names.iter().enumerate().map(|(i, name)| MenuItem::op(name, "", Op::with("node.set_prop", &format!("{}\t{n}\t{prop}\t{i}", r.encode())))).collect()
}

/// The name an Id property points at.
pub fn id_name(d: &Doc, prop: &str, v: Option<usize>) -> Option<String> {
    let i = v?;
    match prop {
        "image" => d.scene.images.get(i).map(|x| x.name.clone()),
        "object" => d.scene.objects.get(i).map(|x| x.name.clone()),
        "node_tree" => d.scene.node_groups.get(i).map(|x| x.name.clone()),
        _ => Some(format!("#{i}")),
    }
}

/// The node's properties as rows.
pub fn prop_rows(l: &mut L, d: &mut Doc, pk: &mut Picker, r: TreeRef, n: usize, key: &str) {
    let Some(node) = r.get(&d.scene).and_then(|t| t.nodes.get(n)).cloned() else { return };
    for (k, (name, p)) in node.props.iter().enumerate() {
        if !update::prop_visible(&node, name) {
            continue;
        }
        let label = enums::prop_label(name);
        let pkey = format!("{key}/{n}/p/{name}");
        match p {
            Prop::Enum(v) => {
                let items = enums::items(&node.idname, name);
                let tref = r.encode();
                let nm = name.clone();
                let list = move || -> Vec<MenuItem> { items.iter().map(|(id, lab)| MenuItem::op(lab, "", Op::with("node.set_prop", &format!("{tref}\t{n}\t{nm}\t{id}")))).collect() };
                l.dropdown_row("", &pkey, enums::label(&node.idname, name, v), &list);
            }
            Prop::Bool(b) => {
                let mut v = *b;
                if l.check("", &label, &mut v) {
                    write_prop(l, d, r, n, k, &label, Prop::Bool(v));
                }
            }
            Prop::Int(i) => {
                let mut v = *i as i64;
                if l.int(&label, &pkey, &mut v, i32::MIN as i64, i32::MAX as i64) {
                    write_prop(l, d, r, n, k, &label, Prop::Int(v as i32));
                }
            }
            Prop::Float(f) => {
                let mut v = *f;
                if l.number(&label, &pkey, &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
                    write_prop(l, d, r, n, k, &label, Prop::Float(v));
                }
            }
            Prop::String(s) => {
                let mut v = s.clone();
                if l.text(&label, &pkey, &mut v) {
                    write_prop(l, d, r, n, k, &label, Prop::String(v));
                }
            }
            Prop::Id(v) => {
                let shown = id_name(d, name, *v).unwrap_or_default();
                let items = id_items(d, r, n, name);
                l.dropdown_row(&label, &pkey, &shown, &move || items.clone());
            }
            Prop::Color(c) => {
                if let Some(rect) = l.color(&label, [c[0], c[1], c[2]]) {
                    *pk = Some((ColorTarget::Node(r, n, NodeColor::Prop(k)), rect));
                }
            }
            Prop::Vector(_) | Prop::Ramp(_) | Prop::Curve(_) => {
                let what = match p {
                    Prop::Ramp(_) => "Color Ramp",
                    Prop::Curve(_) => "Curve",
                    _ => "Vector",
                };
                l.info(&label, &format!("{what} (edit in the node editor)"));
            }
        }
    }
}

/// One input as a property row (or an expandable link).
#[allow(clippy::too_many_arguments)]
fn input_row(l: &mut L, d: &mut Doc, pk: &mut Picker, r: TreeRef, n: usize, i: usize, key: &str, depth: usize) {
    let Some(t) = r.get(&d.scene) else { return };
    let node = &t.nodes[n];
    let s = node.inputs[i].clone();
    let rk = format!("{key}/{n}/i/{}", s.identifier);
    if let Some(link) = t.input_link(n, i).copied() {
        let from = &t.nodes[link.from_node];
        let lab = from.display_label();
        let shown = if from.outputs.iter().filter(|o| o.enabled).count() > 1 { format!("{} | {}", lab, from.outputs[link.from_socket].name) } else { lab };
        let from_node = link.from_node;
        if depth >= 4 {
            l.info(&s.name, &shown);
            return;
        }
        // A linked input expands into the inputs of the node feeding it.
        let title = format!("{}: {}", s.name, shown);
        l.fields.push((title.clone(), Rect::new(l.x, l.y, l.w, 24)));
        l.panel(d, &format!("{rk}/open"), &title, false, &mut |l, d| node_rows(l, d, pk, r, from_node, key, depth + 1));
        return;
    }
    if s.hide_value || !crate::editors::node::layout::has_value(s.ty) {
        l.info(&s.name, if s.ty == SocketType::Shader { "None" } else { "Default" });
        return;
    }
    match (&s.value, s.ty) {
        (Value::Float(f), _) => {
            let (min, max, factor) = crate::editors::node::layout::float_range(&s);
            let angle = s.subtype == Subtype::Angle;
            let mut v = if angle { f.to_degrees() } else { *f };
            let o = NumOpts { step: if factor { (max - min) / 100.0 } else { 0.01 }, min: if angle { f32::NEG_INFINITY } else { min }, max: if angle { f32::INFINITY } else { max }, slider: factor, unit: if angle { "°" } else { "" }, ..NumOpts::default() };
            if l.number(&s.name, &rk, &mut v, o) {
                write_input(l, d, r, n, i, &s.name, Value::Float(if angle { v.to_radians() } else { v }));
            }
        }
        (Value::Int(x), _) => {
            let mut v = *x as i64;
            let (lo, hi) = (if s.min <= f32::MIN { i32::MIN as i64 } else { s.min as i64 }, if s.max >= f32::MAX { i32::MAX as i64 } else { s.max as i64 });
            if l.int(&s.name, &rk, &mut v, lo, hi) {
                write_input(l, d, r, n, i, &s.name, Value::Int(v as i32));
            }
        }
        (Value::Bool(b), _) => {
            let mut v = *b;
            if l.check("", &s.name, &mut v) {
                write_input(l, d, r, n, i, &s.name, Value::Bool(v));
            }
        }
        (Value::Color(c), _) => {
            if let Some(rect) = l.color(&s.name, [c[0], c[1], c[2]]) {
                *pk = Some((ColorTarget::Node(r, n, NodeColor::Input(i)), rect));
            }
        }
        (Value::Vector(vv), _) => {
            for k in 0..3 {
                let mut v = vv[k];
                let lab = if k == 0 { format!("{} X", s.name) } else { ["X", "Y", "Z"][k].to_string() };
                if l.number(&lab, &format!("{rk}/{k}"), &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
                    let mut nv = *vv;
                    nv[k] = v;
                    write_input(l, d, r, n, i, &s.name, Value::Vector(nv));
                }
            }
        }
        (Value::String(st), _) => {
            let mut v = st.clone();
            if l.text(&s.name, &rk, &mut v) {
                write_input(l, d, r, n, i, &s.name, Value::String(v));
            }
        }
        (Value::Id(v), ty) => {
            let prop = match ty {
                SocketType::Image => "image",
                SocketType::Object => "object",
                _ => "",
            };
            let shown = id_name(d, prop, *v).unwrap_or_default();
            l.info(&s.name, &shown);
        }
        _ => l.info(&s.name, ""),
    }
}

/// A node's properties and inputs, with Blender's panels as sub-panels.
pub fn node_rows(l: &mut L, d: &mut Doc, pk: &mut Picker, r: TreeRef, n: usize, key: &str, depth: usize) {
    prop_rows(l, d, pk, r, n, key);
    let Some(node) = r.get(&d.scene).and_then(|t| t.nodes.get(n)).cloned() else { return };
    let mut panels: Vec<String> = Vec::new();
    for (i, s) in node.inputs.iter().enumerate() {
        if !s.enabled || s.ty == SocketType::Virtual {
            continue;
        }
        match &s.panel {
            None => input_row(l, d, pk, r, n, i, key, depth),
            Some(p) => {
                if !panels.contains(p) {
                    panels.push(p.clone());
                }
            }
        }
    }
    for p in panels {
        let pkey = format!("{key}/{n}/panel/{p}");
        l.panel(d, &pkey, &p, false, &mut |l, d| {
            for (i, s) in node.inputs.iter().enumerate() {
                if s.enabled && s.panel.as_deref() == Some(p.as_str()) {
                    input_row(l, d, pk, r, n, i, key, depth);
                }
            }
        });
    }
}

/// The shader types a Surface/Volume menu offers.
fn shader_menu(r: TreeRef, n: Option<usize>) -> Vec<MenuItem> {
    let Some(n) = n else { return vec![MenuItem::Heading("Add a shader in the Shader Editor".into())] };
    registry::node_types()
        .iter()
        .filter(|t| t.category == Category::Shader && t.tree == sex_3d::nodes::TreeKind::Shader)
        .map(|t| MenuItem::op(t.label, "", Op::with("node.replace_surface", &format!("{}\t{n}\t{}", r.encode(), t.idname))))
        .collect()
}

/// The panel for an output's `input` ("Surface", "Volume", "Displacement").
fn output_panel(l: &mut L, d: &mut Doc, pk: &mut Picker, r: TreeRef, out_idname: &str, input: &str, key: &str, open: bool) {
    l.panel(d, &format!("{key}.{}", input.to_lowercase()), input, open, &mut |l, d| {
        let Some(t) = r.get(&d.scene) else { return };
        let Some(o) = t.output_node(out_idname) else {
            l.info(input, "No output node");
            return;
        };
        let Some(si) = t.nodes[o].input(input) else { return };
        match t.input_link(o, si).map(|lk| lk.from_node) {
            Some(src) => {
                let label = t.nodes[src].display_label();
                let menu = shader_menu(r, Some(src));
                let shader = t.nodes[o].inputs[si].ty == SocketType::Shader;
                if shader {
                    l.dropdown_row(input, &format!("{key}/{input}/node"), &label, &move || menu.clone());
                } else {
                    l.info(input, &label);
                }
                node_rows(l, d, pk, r, src, key, 1);
            }
            None => l.info(input, if input == "Displacement" { "Default" } else { "None" }),
        }
    });
}

/// Material tab panels for a material with a node tree.
pub fn material(l: &mut L, d: &mut Doc, pk: &mut Picker, mi: usize) {
    let r = TreeRef::Material(mi);
    output_panel(l, d, pk, r, "ShaderNodeOutputMaterial", "Surface", "material", true);
    output_panel(l, d, pk, r, "ShaderNodeOutputMaterial", "Volume", "material", false);
    output_panel(l, d, pk, r, "ShaderNodeOutputMaterial", "Displacement", "material", false);
}

/// World tab panels for a world with a node tree.
pub fn world(l: &mut L, d: &mut Doc, pk: &mut Picker) {
    let r = TreeRef::World;
    output_panel(l, d, pk, r, "ShaderNodeOutputWorld", "Surface", "world", true);
    output_panel(l, d, pk, r, "ShaderNodeOutputWorld", "Volume", "world", false);
}

/// "Use Nodes" for a material or world without a tree.
pub fn use_nodes_button(l: &mut L, d: &mut Doc, r: TreeRef) {
    if l.button("Use Nodes") {
        d.queue.push(Op::with("node.use_nodes", &r.encode()));
    }
}
