// SPDX-License-Identifier: GPL-3.0-only
//! The Physics and Particles tabs and the Scene tab's Gravity and Rigid
//! Body World panels.
//!
//! Every simulation's settings describe their own panels (sex-3d's
//! [`props`] visitor: Blender's panel layout, labels and limits), so one
//! renderer draws them all: numbers, checkboxes, enum dropdowns, object /
//! collection / vertex group pickers, header checkboxes (Self Collisions,
//! Pressure, ...) and greyed properties. Cache panels get the bake buttons
//! and the cache status.
use super::icons::{self, Icon};
use super::layout::{L, GAP, ROW};
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::physics::{self as phys, Target};
use crate::theme as t;
use crate::ui::{MenuItem, NumOpts, Op};
use sex_3d::modifiers::Modifier;
use sex_3d::physics::props::{self, Entry, Node, RefKind, Value};
use sex_3d::physics::{FluidType, PhysicsModifier};
use sex_3d::scene::ObjectData;

fn queue(d: &mut Doc, id: &str, arg: &str) {
    d.queue.push(Op::with(id, arg));
}

fn set_op(t: Target, id: &str, text: &str) -> Op {
    Op::with("physics.set", &format!("{}\t{id}\t{text}", t.encode()))
}

/// A header checkbox before the panel title; the new value when clicked.
fn check_header(l: &mut L, title: &str, value: bool, r: Rect) -> Option<bool> {
    let mut v = value;
    let tw = l.ui.p.text_width(title) + 24;
    let cr = Rect::new(r.x - 4, r.y, tw.min(r.w + 4), r.h);
    l.ui.checkbox(cr, title, &mut v).then_some(v)
}

fn apply(l: &mut L, d: &mut Doc, t: Target, label: &str, id: &str, v: Value) {
    l.commit(d, label);
    phys::with_props(&mut d.scene, t, |p| props::set_value(p, id, &v));
}

/// Names a reference property can pick from.
fn ref_items(d: &Doc, t: Target, kind: RefKind) -> Vec<String> {
    match kind {
        RefKind::Object => d.scene.objects.iter().map(|o| o.name.clone()).collect(),
        RefKind::Collection => d.scene.collections.iter().map(|c| c.name.clone()).collect(),
        RefKind::VertexGroup => {
            let o = match t {
                Target::RigidBody(o) | Target::Constraint(o) | Target::Field(o) | Target::Modifier(o, _) => o,
                _ => return Vec::new(),
            };
            match d.scene.objects.get(o).map(|o| &o.data) {
                Some(ObjectData::Mesh(m)) => d.scene.meshes.get(*m).map(|m| m.vertex_groups.clone()).unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        RefKind::Text => Vec::new(),
    }
}

/// One property row (three for vectors).
fn entry(l: &mut L, d: &mut Doc, t: Target, e: &Entry) {
    let was = l.inactive;
    l.inactive = was || !e.active;
    let key = format!("{}/{}", t.encode(), e.id);
    let r = e.range.unwrap_or(props::Range::new(f32::NEG_INFINITY, f32::INFINITY, 0.1));
    let opts = NumOpts { step: r.step, min: r.min, max: r.max, unit: r.unit, slider: r.slider, int: false };
    match &e.value {
        Value::F(v) => {
            let mut v = *v;
            if l.number(e.label, &key, &mut v, opts) {
                apply(l, d, t, e.label, e.id, Value::F(v));
            }
        }
        Value::I(v) => {
            let mut f = *v as f32;
            if l.number(e.label, &key, &mut f, NumOpts { step: 1.0, int: true, ..opts }) {
                apply(l, d, t, e.label, e.id, Value::I(f.round() as i32));
            }
        }
        Value::B(v) => {
            let mut b = *v;
            if l.check("", e.label, &mut b) {
                apply(l, d, t, e.label, e.id, Value::B(b));
            }
        }
        Value::E(i, _) => {
            let shown = e.items.get(*i).map_or("", |x| x.1);
            let items = e.items.clone();
            let id = e.id;
            l.dropdown_row(e.label, &key, shown, &|| items.iter().map(|(ident, label)| MenuItem::op(label, "", set_op(t, id, ident))).collect());
        }
        Value::V3(v) => {
            let color = r.min == 0.0 && r.max == 1.0 && r.unit.is_empty() && !r.slider && r.step == 0.01;
            let axes = if color { ["R", "G", "B"] } else { ["X", "Y", "Z"] };
            for k in 0..3 {
                let mut x = v[k];
                let label = if k == 0 { format!("{} {}", e.label, axes[0]) } else { axes[k].to_string() };
                if l.number(&label, &format!("{key}{k}"), &mut x, opts) {
                    let mut nv = *v;
                    nv[k] = x;
                    apply(l, d, t, e.label, e.id, Value::V3(nv));
                }
            }
        }
        Value::S(s) => match e.reference {
            Some(kind) if kind != RefKind::Text => {
                let names = ref_items(d, t, kind);
                let id = e.id;
                let shown = if s.is_empty() { "" } else { s.as_str() };
                l.dropdown_row(e.label, &key, shown, &|| {
                    let mut items = vec![MenuItem::op("(None)", "", set_op(t, id, "none"))];
                    items.extend(names.iter().map(|n| MenuItem::op(n, "", set_op(t, id, n))));
                    items
                });
            }
            _ => {
                let mut s = s.clone();
                if l.text(e.label, &key, &mut s) {
                    apply(l, d, t, e.label, e.id, Value::S(s));
                }
            }
        },
    }
    l.inactive = was;
}

/// Whether `t` is a fluid domain (its Cache panel bakes with the fluid
/// operators).
fn is_fluid_domain(d: &Doc, t: Target) -> bool {
    match t {
        Target::Modifier(o, s) => matches!(d.scene.objects.get(o).and_then(|o| o.modifiers.get(s)), Some(Modifier::Physics(PhysicsModifier::Fluid(f))) if f.fluid_type == FluidType::Domain),
        _ => false,
    }
}

/// The bake buttons and status under a Cache panel's settings.
fn cache_buttons(l: &mut L, d: &mut Doc, t: Target) {
    if is_fluid_domain(d, t) {
        let Some(key) = t.cache_key(&d.scene) else { return };
        let (info, baked, outdated, _) = d.scene.physics.caches.status(&key);
        l.note(&if outdated { format!("{info} (outdated)") } else { info });
        if baked {
            if l.button("Free All") {
                queue(d, "fluid.free_all", &key);
            }
        } else if l.button("Bake All") {
            queue(d, "fluid.bake_all", &key);
        }
        return;
    }
    let Some(key) = t.cache_key(&d.scene) else {
        l.note("Not simulated with the current settings");
        return;
    };
    let (info, baked, outdated, _) = d.scene.physics.caches.status(&key);
    let status = if outdated { format!("{info} (outdated)") } else { info };
    l.note(&status);
    if baked {
        if l.button("Delete Bake") {
            queue(d, "ptcache.free_bake", &key);
        }
    } else if l.button("Bake") {
        queue(d, "ptcache.bake", &key);
    }
    if l.button("Bake All Dynamics") {
        queue(d, "ptcache.bake_all", "");
    }
    if l.button("Delete All Bakes") {
        queue(d, "ptcache.free_bake_all", "");
    }
}

/// Draw a collected settings tree; `skip` names top-level properties drawn
/// elsewhere (a header checkbox).
pub fn nodes(l: &mut L, d: &mut Doc, t: Target, list: &[Node], skip: &[&str]) {
    for n in list {
        match n {
            Node::Prop(e) => {
                if !skip.contains(&e.id) {
                    entry(l, d, t, e);
                }
            }
            Node::Panel { key, title, open, toggle, children } => {
                let pkey = format!("physics.{key}");
                let is_cache = matches!(*key, "cache" | "fluid_cache");
                let mut body = |l: &mut L, d: &mut Doc| {
                    nodes(l, d, t, children, &[]);
                    if is_cache {
                        cache_buttons(l, d, t);
                    }
                };
                match toggle {
                    Some((id, on)) => {
                        let mut hit = None;
                        let on = *on;
                        l.panel_with(d, &pkey, "", *open, &mut |l, _, r| hit = check_header(l, title, on, r), &mut body);
                        if let Some(v) = hit {
                            apply(l, d, t, title, id, Value::B(v));
                        }
                    }
                    None => {
                        l.panel(d, &pkey, title, *open, &mut body);
                    }
                }
            }
        }
    }
}

fn tree(d: &mut Doc, t: Target) -> Option<Vec<Node>> {
    phys::with_props(&mut d.scene, t, props::tree)
}

/// A top-level panel holding a simulation's settings tree.
fn sim_panel(l: &mut L, d: &mut Doc, key: &str, title: &str, t: Target) {
    let Some(list) = tree(d, t) else { return };
    l.panel(d, key, title, true, &mut |l, d| nodes(l, d, t, &list, &[]));
}

/// Scene > Gravity.
pub fn gravity_panel(l: &mut L, d: &mut Doc) {
    let on = d.scene.physics.gravity.use_gravity;
    let mut hit = None;
    l.panel_with(d, "scene.gravity", "", true, &mut |l, _, r| hit = check_header(l, "Gravity", on, r), &mut |l, d| {
        let was = l.inactive;
        l.inactive = !d.scene.physics.gravity.use_gravity;
        for (i, n) in ["Gravity X", "Y", "Z"].iter().enumerate() {
            let mut v = d.scene.physics.gravity.gravity[i];
            if l.number(n, &format!("gravity{i}"), &mut v, NumOpts { step: 0.1, unit: "m/s²", ..NumOpts::default() }) {
                l.commit(d, "Gravity");
                d.scene.physics.gravity.gravity[i] = v;
            }
        }
        l.inactive = was;
    });
    if let Some(v) = hit {
        l.commit(d, "Gravity");
        d.scene.physics.gravity.use_gravity = v;
    }
}

/// Scene > Rigid Body World.
pub fn world_panel(l: &mut L, d: &mut Doc) {
    let Some(on) = d.scene.physics.rigidbody_world.as_ref().map(|w| w.enabled) else {
        l.panel(d, "scene.rigid_body", "Rigid Body World", false, &mut |l, d| {
            if l.button("Add Rigid Body World") {
                queue(d, "rigidbody.world_add", "");
            }
        });
        return;
    };
    let list = tree(d, Target::World).unwrap_or_default();
    let mut hit = None;
    l.panel_with(d, "scene.rigid_body", "", false, &mut |l, _, r| hit = check_header(l, "Rigid Body World", on, r), &mut |l, d| {
        if l.button("Remove Rigid Body World") {
            queue(d, "rigidbody.world_remove", "");
        }
        let was = l.inactive;
        l.inactive = was || !on;
        nodes(l, d, Target::World, &list, &["enabled"]);
        l.inactive = was;
    });
    if let Some(v) = hit {
        apply(l, d, Target::World, "Rigid Body World", "enabled", Value::B(v));
    }
}

/// The Physics tab: the Enable Physics For buttons, then each enabled
/// simulation's panels.
pub fn physics(l: &mut L, d: &mut Doc) {
    l.space(4);
    let Some(o) = d.active() else {
        l.note("No active object");
        return;
    };
    l.panel(d, "physics.enable", "Enable Physics For:", true, &mut |l, d| {
        for pair in phys::KINDS.chunks(2) {
            if !l.band(ROW) {
                continue;
            }
            let (x, w) = l.inner();
            let half = (w - 4) / 2;
            for (i, (kind, label)) in pair.iter().enumerate() {
                let r = Rect::new(x + i as i32 * (half + 4), l.y, half, ROW);
                l.fields.push((label.to_string(), r));
                let ob = &d.scene.objects[o];
                if !phys::kind_allowed(ob, kind) || l.inactive {
                    l.ui.p.round_rect(r, 4, 0x3a3a3a);
                    l.ui.p.label(r.shrink(4), label, t::TEXT_DIM, Align::Center);
                    continue;
                }
                let on = phys::has_kind(ob, kind);
                if l.ui.button(r, label) {
                    queue(d, "physics.toggle", kind);
                }
                if on {
                    icons::draw(l.ui.p, Icon::Cross, r.x + 12, r.y + ROW / 2);
                }
            }
            l.y += ROW + GAP;
        }
    });
    if d.scene.objects[o].physics.field.is_some() {
        sim_panel(l, d, "physics.field", "Force Fields", Target::Field(o));
    }
    let slots: Vec<(usize, &'static str)> = d.scene.objects[o]
        .modifiers
        .iter()
        .enumerate()
        .filter_map(|(k, m)| match m {
            Modifier::Physics(PhysicsModifier::Collision(_)) => Some((k, "Collision")),
            Modifier::Physics(PhysicsModifier::Cloth(_)) => Some((k, "Cloth")),
            Modifier::Physics(PhysicsModifier::SoftBody(_)) => Some((k, "Soft Body")),
            Modifier::Physics(PhysicsModifier::Fluid(_)) => Some((k, "Fluid")),
            Modifier::Physics(PhysicsModifier::DynamicPaint(_)) => Some((k, "Dynamic Paint")),
            _ => None,
        })
        .collect();
    for (k, title) in slots {
        sim_panel(l, d, &format!("physics.mod.{title}"), title, Target::Modifier(o, k));
    }
    if d.scene.objects[o].physics.rigid_body.is_some() {
        sim_panel(l, d, "physics.rigid_body", "Rigid Body", Target::RigidBody(o));
    }
    if d.scene.objects[o].physics.rigid_body_constraint.is_some() {
        sim_panel(l, d, "physics.rigid_body_constraint", "Rigid Body Constraint", Target::Constraint(o));
    }
}

/// The Particles tab: the particle system list with + and -, then the
/// active system's settings.
pub fn particles(l: &mut L, d: &mut Doc) {
    l.space(4);
    let Some(o) = d.active() else {
        l.note("No active object");
        return;
    };
    let slots = phys::particle_slots(&d.scene.objects[o]);
    let rows: Vec<(Icon, String)> = slots
        .iter()
        .filter_map(|&k| match &d.scene.objects[o].modifiers[k] {
            Modifier::Physics(PhysicsModifier::ParticleSystem(p)) => Some((Icon::Particles, p.name.clone())),
            _ => None,
        })
        .collect();
    let active = phys::active_particle(d, o);
    let (b, picked) = l.list_box(&rows, active, 3);
    match b {
        1 => queue(d, "object.particle_system_add", ""),
        2 => queue(d, "object.particle_system_remove", ""),
        _ => {}
    }
    if let Some(p) = picked {
        queue(d, "physics.particle_active", &p.to_string());
    }
    let Some(&slot) = slots.get(active) else { return };
    let (mut name, mut sname) = match &d.scene.objects[o].modifiers[slot] {
        Modifier::Physics(PhysicsModifier::ParticleSystem(p)) => (p.name.clone(), p.settings.name.clone()),
        _ => return,
    };
    if l.text("Name", "psys_name", &mut name) {
        l.commit(d, "Rename Particle System");
        if let Modifier::Physics(PhysicsModifier::ParticleSystem(p)) = &mut d.scene.objects[o].modifiers[slot] {
            p.name = name;
        }
    }
    if l.text("Settings", "psys_settings", &mut sname) {
        l.commit(d, "Rename Particle Settings");
        if let Modifier::Physics(PhysicsModifier::ParticleSystem(p)) = &mut d.scene.objects[o].modifiers[slot] {
            p.settings.name = sname;
        }
    }
    let t = Target::Modifier(o, slot);
    if let Some(list) = tree(d, t) {
        nodes(l, d, t, &list, &[]);
    }
}
