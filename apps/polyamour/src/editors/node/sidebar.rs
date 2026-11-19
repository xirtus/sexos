// SPDX-License-Identifier: GPL-3.0-only
//! The node editor's sidebar (N): the Node tab (name, label, colour and the
//! active node's properties) and the Group tab (the interface sockets of the
//! group being edited), drawn with the Properties editor's panels.
use super::State;
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::editors::properties::layout::L;
use crate::editors::properties::nodepanel;
use crate::editors::properties::picker::Picker;
use crate::editors::properties::ColorTarget;
use crate::input::Input;
use crate::nodetree::{NodeColor, TreeRef};
use crate::theme as t;
use crate::ui::{MenuItem, NumOpts, Ui};
use sex_3d::nodes::{edit, SocketType, Value};

const TYPES: [(SocketType, &str); 8] = [
    (SocketType::Float, "Float"),
    (SocketType::Int, "Integer"),
    (SocketType::Bool, "Boolean"),
    (SocketType::Vector, "Vector"),
    (SocketType::Color, "Color"),
    (SocketType::Shader, "Shader"),
    (SocketType::Geometry, "Geometry"),
    (SocketType::String, "String"),
];

pub fn draw(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect, tref: TreeRef) {
    ui.p.fill(r, t::REGION);
    ui.p.fill(Rect::new(r.x, r.y, 1, r.h), 0x262626);
    ui.p.push_clip(r);
    let group = tref.group();
    let tabs: Vec<&str> = if group.is_some() { vec!["Node", "Group"] } else { vec!["Node"] };
    s.side_tab = s.side_tab.min(tabs.len() - 1);
    let mut tab = s.side_tab;
    ui.segmented(Rect::new(r.x + 8, r.y + 6, r.w - 16, 20), &tabs, &mut tab);
    s.side_tab = tab;
    let content = Rect::new(r.x, r.y + 32, r.w, r.h - 32);
    ui.p.push_clip(content);
    let mut pk: nodepanel::Picker = None;
    let mut l = L {
        ui: &mut *ui,
        open: &mut s.side_open,
        x: content.x + 6,
        w: content.w - 12,
        y: content.y + 4 - s.side_scroll,
        query: String::new(),
        dry: false,
        hits: 0,
        depth: 0,
        inactive: false,
        prefix: format!("nodeside{}", s.id),
        undo_for: s.side_undo,
        fields: Vec::new(),
    };
    let top = l.y;
    if tab == 0 {
        node_tab(&mut l, d, &mut pk, tref);
    } else if let Some(g) = group {
        group_tab(&mut l, d, s.id, &mut s.iface_sel, g);
    }
    let h = l.y - top;
    s.side_undo = if l.ui.st.active.is_none() { None } else { l.undo_for };
    for (k, rr) in core::mem::take(&mut l.fields) {
        s.fields.push((format!("sidebar/{k}"), rr));
    }
    if let Some(Input::Wheel { x, y, steps, .. }) = ui.input {
        if content.contains(x, y) {
            ui.input = None;
            s.side_scroll = (s.side_scroll - steps * 40).clamp(0, (h - content.h).max(0));
        }
    }
    ui.p.pop_clip();
    ui.p.pop_clip();
    if let Some((target, anchor)) = pk {
        s.picker = Some(Picker::new(target, anchor, d));
    }
}

fn node_tab(l: &mut L, d: &mut Doc, pk: &mut nodepanel::Picker, tref: TreeRef) {
    let Some(t) = tref.get(&d.scene) else { return };
    let Some(a) = t.active.filter(|&a| a < t.nodes.len()) else {
        l.note("No active node");
        return;
    };
    l.panel(d, "node", "Node", true, &mut |l, d| {
        let Some(node) = tref.get(&d.scene).and_then(|t| t.nodes.get(a)).cloned() else { return };
        let mut name = node.name.clone();
        if l.text("Name", "name", &mut name) && !name.trim().is_empty() {
            l.commit(d, "Rename Node");
            if let Some(t) = tref.get_mut(&mut d.scene) {
                let nm = if t.nodes.iter().enumerate().any(|(i, n)| i != a && n.name == name.trim()) { t.unique_name(name.trim()) } else { name.trim().to_string() };
                t.nodes[a].name = nm;
            }
        }
        let mut label = node.label.clone();
        if l.text("Label", "label", &mut label) {
            l.commit(d, "Node Label");
            if let Some(t) = tref.get_mut(&mut d.scene) {
                t.nodes[a].label = label;
            }
        }
    });
    l.panel(d, "node.color", "Color", false, &mut |l, d| {
        let Some(node) = tref.get(&d.scene).and_then(|t| t.nodes.get(a)).cloned() else { return };
        let mut on = node.color.is_some();
        if l.check("", "Use Custom Color", &mut on) {
            l.commit(d, "Node Color");
            if let Some(t) = tref.get_mut(&mut d.scene) {
                t.nodes[a].color = if on { Some([0.35, 0.35, 0.35]) } else { None };
            }
        }
        if let Some(c) = node.color {
            if let Some(rr) = l.color("Color", c) {
                *pk = Some((ColorTarget::Node(tref, a, NodeColor::Custom), rr));
            }
        }
    });
    l.panel(d, "node.properties", "Properties", true, &mut |l, d| {
        nodepanel::node_rows(l, d, pk, tref, a, "side", 2);
    });
}

fn group_tab(l: &mut L, d: &mut Doc, sid: u64, sel: &mut Option<(bool, usize)>, g: usize) {
    let mail = |op: &str, arg: &str| crate::anim::mail(op, sid, arg);
    l.panel(d, "group.interface", "Interface", true, &mut |l, d| {
        let Some(gr) = d.scene.node_groups.get(g).cloned() else { return };
        for (input, list, title) in [(false, &gr.outputs, "Outputs"), (true, &gr.inputs, "Inputs")] {
            if l.band(20) {
                let (x, w) = l.inner();
                l.ui.p.label(Rect::new(x, l.y, w - 60, 20), title, t::TEXT_DIM, Align::Left);
                let add = Rect::new(x + w - 22, l.y, 22, 20);
                if l.ui.button(add, "+") {
                    d.queue.push(mail("node.interface_add", if input { "in" } else { "out" }));
                }
                l.y += 22;
            }
            for (i, so) in list.iter().enumerate() {
                if !l.band(20) {
                    continue;
                }
                let (x, w) = l.inner();
                let rr = Rect::new(x, l.y, w, 20);
                let on = *sel == Some((input, i));
                if on {
                    l.ui.p.round_rect(rr, 3, t::OUTLINER_ACTIVE);
                }
                l.ui.p.circle((rr.x + 8) as f32, (rr.y + 10) as f32, 4.0, so.ty.color());
                l.ui.p.label(Rect::new(rr.x + 18, rr.y, rr.w - 20, rr.h), &so.name, t::TEXT, Align::Left);
                l.fields.push((format!("iface/{}/{i}", if input { "in" } else { "out" }), rr));
                if l.ui.clicked(rr) {
                    *sel = Some((input, i));
                }
                l.y += 21;
            }
        }
        let Some((input, i)) = *sel else { return };
        let list = if input { &gr.inputs } else { &gr.outputs };
        let Some(so) = list.get(i).cloned() else { return };
        let side = if input { "in" } else { "out" };
        l.space(6);
        if l.band(20) {
            let (x, w) = l.inner();
            let bw = (w - 6) / 3;
            if l.ui.button(Rect::new(x, l.y, bw, 20), "Remove") {
                d.queue.push(mail("node.interface_remove", &format!("{side}\t{i}")));
            }
            if l.ui.button(Rect::new(x + bw + 3, l.y, bw, 20), "Up") {
                d.queue.push(mail("node.interface_move", &format!("{side}\t{i}\t-1")));
            }
            if l.ui.button(Rect::new(x + 2 * (bw + 3), l.y, bw, 20), "Down") {
                d.queue.push(mail("node.interface_move", &format!("{side}\t{i}\t1")));
            }
            l.y += 24;
        }
        let mut name = so.name.clone();
        if l.text("Name", "iface_name", &mut name) {
            l.commit(d, "Rename Socket");
            set_iface(d, g, input, i, |x| x.name = name.clone());
        }
        let ty_label = TYPES.iter().find(|x| x.0 == so.ty).map(|x| x.1).unwrap_or("Other");
        let items: Vec<MenuItem> = TYPES
            .iter()
            .map(|(_, lab)| MenuItem::op(lab, "", crate::ui::Op::with("node.interface_type", &format!("{g}\t{side}\t{i}\t{lab}"))))
            .collect();
        l.dropdown_row("Type", "iface_type", ty_label, &move || items.clone());
        match so.default {
            Value::Float(f) => {
                let mut v = f;
                if l.number("Default", "iface_def", &mut v, NumOpts { step: 0.01, ..NumOpts::default() }) {
                    l.commit(d, "Socket Default");
                    set_iface(d, g, input, i, |x| x.default = Value::Float(v));
                }
                let (mut lo, mut hi) = (if so.min <= f32::MIN { -10000.0 } else { so.min }, if so.max >= f32::MAX { 10000.0 } else { so.max });
                if l.number("Min", "iface_min", &mut lo, NumOpts { step: 0.01, ..NumOpts::default() }) {
                    l.commit(d, "Socket Min");
                    set_iface(d, g, input, i, |x| x.min = lo);
                }
                if l.number("Max", "iface_max", &mut hi, NumOpts { step: 0.01, ..NumOpts::default() }) {
                    l.commit(d, "Socket Max");
                    set_iface(d, g, input, i, |x| x.max = hi);
                }
            }
            Value::Int(n) => {
                let mut v = n as i64;
                if l.int("Default", "iface_def", &mut v, i32::MIN as i64, i32::MAX as i64) {
                    l.commit(d, "Socket Default");
                    set_iface(d, g, input, i, |x| x.default = Value::Int(v as i32));
                }
            }
            Value::Bool(b) => {
                let mut v = b;
                if l.check("Default", "", &mut v) {
                    l.commit(d, "Socket Default");
                    set_iface(d, g, input, i, |x| x.default = Value::Bool(v));
                }
            }
            _ => {}
        }
    });
}

/// Change an interface socket and refresh every group node using it.
fn set_iface(d: &mut Doc, g: usize, input: bool, i: usize, f: impl FnOnce(&mut sex_3d::nodes::InterfaceSocket)) {
    let Some(gr) = d.scene.node_groups.get_mut(g) else { return };
    let list = if input { &mut gr.inputs } else { &mut gr.outputs };
    if let Some(so) = list.get_mut(i) {
        f(so);
    }
    let groups = d.scene.node_groups.clone();
    crate::nodetree::each_tree_mut(&mut d.scene, |t, own| edit::sync_group_nodes(t, &groups, own));
    d.touch();
}
