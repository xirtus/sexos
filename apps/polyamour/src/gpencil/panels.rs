// SPDX-License-Identifier: GPL-3.0-only
//! Properties tabs of a Grease Pencil object: Object Data (layers, onion
//! skinning, vertex groups), Material slots and styles, the Modifiers stack
//! and Visual Effects. Field lists come from the settings visitors
//! (see `ui.rs`).
use super::ui::{self, GpOwner};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::editors::properties::icons::{self, Icon};
use crate::editors::properties::layout::{L, ROW};
use crate::editors::properties::Ctx;
use crate::theme as t;
use crate::ui::{MenuItem, Op};

fn gp_of(d: &Doc, c: &Ctx) -> Option<(usize, usize)> {
    let o = c.obj?;
    match d.scene.objects.get(o)?.data {
        sex_3d::scene::ObjectData::GreasePencil(g) if g < d.scene.grease_pencils.len() => Some((o, g)),
        _ => None,
    }
}

fn picker(c: &mut Ctx, p: Option<(crate::editors::properties::ColorTarget, Rect)>) {
    if p.is_some() {
        c.open_picker = p;
    }
}

/// A row of small buttons; returns the index clicked.
fn buttons(l: &mut L, labels: &[&str]) -> Option<usize> {
    if !l.band(ROW) {
        return None;
    }
    let (x, w) = l.inner();
    let n = labels.len() as i32;
    let bw = (w - (n - 1) * 2) / n.max(1);
    let mut hit = None;
    for (i, lab) in labels.iter().enumerate() {
        let r = Rect::new(x + i as i32 * (bw + 2), l.y, bw, ROW);
        l.fields.push((lab.to_string(), r));
        if !l.inactive && l.ui.button(r, lab) {
            hit = Some(i);
        }
    }
    l.y += ROW + crate::editors::properties::layout::GAP;
    hit
}

/// Object Data tab of a Grease Pencil object.
pub fn data(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some((_, g)) = gp_of(d, c) else { return };
    let name = d.scene.grease_pencil_names.get(g).cloned().unwrap_or_default();
    if let Some(n) = l.id_block(Icon::GreasePencilData, &name, Some("gp_name")) {
        let n = n.trim().to_string();
        if !n.is_empty() {
            d.push_undo("Rename Grease Pencil");
            let n = sex_3d::scene::unique_name(&n, |x| d.scene.grease_pencil_names.iter().enumerate().any(|(i, m)| i != g && m == x));
            d.scene.grease_pencil_names[g] = n;
        }
    }
    let mut pk = None;
    l.panel(d, "gp.layers", "Layers", true, &mut |l, d| {
        let gpd = &d.scene.grease_pencils[g];
        let n = gpd.layers.len();
        // Top layer first, as in the layer list of the 2D editors.
        let rows: Vec<(Icon, String)> = gpd
            .layers
            .iter()
            .rev()
            .map(|ly| {
                let mut s = ly.name.clone();
                if ly.hide {
                    s.push_str("  (hidden)");
                }
                if ly.lock {
                    s.push_str("  (locked)");
                }
                (Icon::GreasePencilData, s)
            })
            .collect();
        let act = n.saturating_sub(1).saturating_sub(gpd.active_layer);
        let (b, picked) = l.list_box(&rows, act, 4);
        match b {
            1 => d.queue.push(Op::new("grease_pencil.layer_add")),
            2 => d.queue.push(Op::new("grease_pencil.layer_remove")),
            _ => {}
        }
        if let Some(p) = picked {
            d.queue.push(Op::with("grease_pencil.layer_active", &(n - 1 - p).to_string()));
        }
        match buttons(l, &["Up", "Down", "Duplicate", "Merge Down", "Group"]) {
            Some(0) => d.queue.push(Op::with("grease_pencil.layer_move", "UP")),
            Some(1) => d.queue.push(Op::with("grease_pencil.layer_move", "DOWN")),
            Some(2) => d.queue.push(Op::new("grease_pencil.layer_duplicate")),
            Some(3) => d.queue.push(Op::new("grease_pencil.layer_merge")),
            Some(4) => d.queue.push(Op::new("grease_pencil.layer_group_add")),
            _ => {}
        }
        let gpd = &d.scene.grease_pencils[g];
        let Some(li) = (gpd.active_layer < n).then_some(gpd.active_layer) else { return };
        let mut lname = gpd.layers[li].name.clone();
        if l.text("Name", "gp_layer_name", &mut lname) && !lname.trim().is_empty() {
            l.commit(d, "Rename Layer");
            let gpd = &mut d.scene.grease_pencils[g];
            let n = sex_3d::scene::unique_name(lname.trim(), |x| gpd.layers.iter().enumerate().any(|(i, ly)| i != li && ly.name == x));
            gpd.layers[li].name = n;
        }
        pk = ui::draw(l, d, GpOwner::Layer(g, li), &[]);
    });
    picker(c, pk);
    if !d.scene.grease_pencils[g].groups.is_empty() {
        l.panel(d, "gp.groups", "Layer Groups", false, &mut |l, d| {
            let n = d.scene.grease_pencils[g].groups.len();
            for gi in 0..n {
                let name = d.scene.grease_pencils[g].groups[gi].name.clone();
                l.note(&name);
                ui::draw(l, d, GpOwner::Group(g, gi), &["expanded"]);
            }
        });
    }
    pk = None;
    l.panel(d, "gp.onion", "Onion Skinning", true, &mut |l, d| {
        pk = ui::draw(l, d, GpOwner::Data(g), &[]);
    });
    picker(c, pk);
    l.panel(d, "gp.vgroups", "Vertex Groups", false, &mut |l, d| {
        let gpd = &d.scene.grease_pencils[g];
        let rows: Vec<(Icon, String)> = gpd.vertex_groups.iter().map(|n| (Icon::VertexGroup, n.clone())).collect();
        let (b, picked) = l.list_box(&rows, gpd.active_vertex_group, 3);
        match b {
            1 => d.queue.push(Op::new("grease_pencil.vertex_group_add")),
            2 => d.queue.push(Op::new("grease_pencil.vertex_group_remove")),
            _ => {}
        }
        if let Some(p) = picked {
            d.queue.push(Op::with("grease_pencil.vertex_group_active", &p.to_string()));
        }
        if d.mode == crate::doc::Mode::Edit {
            match buttons(l, &["Assign", "Remove"]) {
                Some(0) => d.queue.push(Op::new("grease_pencil.vertex_group_assign")),
                Some(1) => d.queue.push(Op::new("grease_pencil.vertex_group_remove_from")),
                _ => {}
            }
            l.slider("Weight", "gp_vg_weight", &mut d.gp.weight_value, 0.0, 1.0);
        }
    });
}

/// Material tab of a Grease Pencil object: slots and the active style.
pub fn material(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some((_, g)) = gp_of(d, c) else { return };
    let gpd = &d.scene.grease_pencils[g];
    let rows: Vec<(Icon, String)> = gpd.materials.iter().map(|m| (Icon::Material, m.name.clone())).collect();
    let act = gpd.active_material;
    l.space(4);
    let (b, picked) = l.list_box(&rows, act, 4);
    match b {
        1 => d.queue.push(Op::new("grease_pencil.material_slot_add")),
        2 => d.queue.push(Op::new("grease_pencil.material_slot_remove")),
        _ => {}
    }
    if let Some(p) = picked {
        d.queue.push(Op::with("grease_pencil.material_active", &p.to_string()));
    }
    if crate::gpencil::in_mode(d) && d.mode == crate::doc::Mode::Edit {
        if buttons(l, &["Assign", "Select"]) == Some(0) {
            d.queue.push(Op::new("grease_pencil.stroke_material_set"));
        }
    }
    let gpd = &d.scene.grease_pencils[g];
    let Some(mi) = (act < gpd.materials.len()).then_some(act) else { return };
    let mut name = gpd.materials[mi].name.clone();
    if let Some(n) = l.id_block(Icon::Material, &name, Some("gp_mat_name")) {
        name = n.trim().to_string();
        if !name.is_empty() {
            d.push_undo("Rename Material");
            d.scene.grease_pencils[g].materials[mi].name = name;
        }
    }
    let mut pk = None;
    l.panel(d, "gp.material", "Surface", true, &mut |l, d| {
        pk = ui::draw(l, d, GpOwner::Material(g, mi), &[]);
    });
    picker(c, pk);
}

/// One stack entry's header: name, viewport and render toggles, menu, X.
/// Returns (toggle viewport, toggle render, menu at, remove).
fn stack_header(l: &mut L, r: Rect, icon: Icon, title: &str, vp: bool, rd: bool) -> (bool, bool, Option<Rect>, bool) {
    icons::draw(l.ui.p, icon, r.x + 6, r.y + r.h / 2);
    let xr = Rect::new(r.right() - 20, r.y, 20, r.h);
    let mr = Rect::new(xr.x - 26, r.y, 24, r.h);
    let cam = Rect::new(mr.x - 26, r.y, 24, r.h);
    let vpr = Rect::new(cam.x - 25, r.y, 24, r.h);
    let nr = Rect::new(r.x + 16, r.y, (vpr.x - 4 - r.x - 16).max(10), r.h);
    l.ui.p.round_rect(nr, 4, t::BUTTON_DARK);
    l.ui.p.label(Rect::new(nr.x + 6, nr.y, nr.w - 8, nr.h), title, t::TEXT, Align::Left);
    let mut out = (false, false, None, false);
    for (rr, on, which) in [(vpr, vp, 0), (cam, rd, 1)] {
        if on {
            l.ui.p.round_rect(rr, 4, t::SELECTED);
        } else if l.ui.hovered(rr) {
            l.ui.p.round_rect(rr, 4, t::BUTTON_HOVER);
        }
        if l.ui.clicked(rr) {
            if which == 0 {
                out.0 = true;
            } else {
                out.1 = true;
            }
        }
    }
    icons::draw(l.ui.p, Icon::Monitor, vpr.x + 12, vpr.y + vpr.h / 2);
    icons::draw(l.ui.p, if rd { Icon::RenderOn } else { Icon::RenderOff }, cam.x + 12, cam.y + cam.h / 2);
    if l.ui.hovered(mr) {
        l.ui.p.round_rect(mr, 4, t::BUTTON_HOVER);
    }
    icons::draw(l.ui.p, Icon::Down, mr.x + 12, mr.y + mr.h / 2);
    if l.ui.clicked(mr) {
        out.2 = Some(mr);
    }
    if l.ui.hovered(xr) {
        l.ui.p.round_rect(xr, 4, t::BUTTON_HOVER);
    }
    icons::draw(l.ui.p, Icon::Cross, xr.x + 10, xr.y + xr.h / 2);
    if l.ui.clicked(xr) {
        out.3 = true;
    }
    out
}

fn add_row(l: &mut L, label: &str, key: &str, icon: Icon, menu: Vec<MenuItem>) {
    l.space(2);
    if l.band(ROW) {
        let (x, w) = l.inner();
        let r = Rect::new(x, l.y, w, ROW);
        l.fields.push((key.into(), r));
        if l.ui.dropdown(r, &format!("     {label}")) {
            l.ui.open_menu(r, "", menu);
        }
        icons::draw(l.ui.p, icon, x + 12, l.y + ROW / 2);
        l.y += ROW + 8;
    }
}

/// The Modifiers tab of a Grease Pencil object.
pub fn modifiers(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some((o, _)) = gp_of(d, c) else { return };
    add_row(l, "Add Modifier", "add_modifier", Icon::Modifier, super::modifier_menu());
    stack(l, d, c, o, false);
}

/// The Visual Effects tab.
pub fn effects(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some((o, _)) = gp_of(d, c) else {
        l.note("Visual effects need a Grease Pencil object");
        return;
    };
    add_row(l, "Add Effect", "add_effect", Icon::ShaderFx, super::effect_menu());
    stack(l, d, c, o, true);
}

fn stack(l: &mut L, d: &mut Doc, c: &mut Ctx, o: usize, fx: bool) {
    let n = if fx { d.scene.objects[o].shader_fx.len() } else { d.scene.objects[o].gp_modifiers.len() };
    let oname = d.scene.objects[o].name.clone();
    let prefix = if fx { "object.shaderfx" } else { "object.gpencil_modifier" };
    for i in 0..n {
        let ob = &d.scene.objects[o];
        let (name, vp, rd) = if fx {
            let e = &ob.shader_fx[i];
            (e.name.clone(), e.show_viewport, e.show_render)
        } else {
            let m = &ob.gp_modifiers[i];
            (m.name.clone(), m.show_viewport, m.show_render)
        };
        let owner = if fx { GpOwner::Effect(o, i) } else { GpOwner::Modifier(o, i) };
        let key = format!("{}/{oname}/{i}/{name}", if fx { "fx" } else { "gpmod" });
        let mut hdr = (false, false, None, false);
        let mut pk = None;
        l.panel_with(
            d,
            &key,
            "",
            true,
            &mut |l, _, r| hdr = stack_header(l, r, if fx { Icon::ShaderFx } else { Icon::Modifier }, &name, vp, rd),
            &mut |l, d| {
                let was = l.inactive;
                l.inactive = was || !vp;
                pk = ui::draw(l, d, owner, &["show_viewport", "show_render"]);
                l.inactive = was;
            },
        );
        picker(c, pk);
        let (tv, tr, menu, remove) = hdr;
        if tv || tr {
            let key = if tv { "show_viewport" } else { "show_render" };
            let now = if tv { vp } else { rd };
            ui::set_pairs(d, owner, vec![(key.into(), (!now as u8).to_string())], true);
        }
        if remove {
            d.queue.push(Op::with(&format!("{prefix}_remove"), &i.to_string()));
        }
        if let Some(r) = menu {
            let mut items = Vec::new();
            if !fx {
                items.push(MenuItem::op("Apply", "Ctrl A", Op::with("object.gpencil_modifier_apply", &i.to_string())));
            }
            items.push(MenuItem::op("Delete", "X", Op::with(&format!("{prefix}_remove"), &i.to_string())));
            items.push(MenuItem::Separator);
            items.push(MenuItem::op("Move Up", "", Op::with(&format!("{prefix}_move"), &format!("{i}:UP"))));
            items.push(MenuItem::op("Move Down", "", Op::with(&format!("{prefix}_move"), &format!("{i}:DOWN"))));
            l.ui.open_menu(r, "", items);
        }
    }
}
