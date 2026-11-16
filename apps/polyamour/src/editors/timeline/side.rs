// SPDX-License-Identifier: GPL-3.0-only
//! The Graph Editor's and Drivers editor's sidebar (N): the F-Curve tab
//! (active curve and active keyframe), the Modifiers tab (Cycles, Noise)
//! and the Drivers tab (driver type, expression, variables).
use super::{Row, State};
use crate::anim::{self, CycleMode, DriverType, Easing, FModifier, HandleType, Interp, KeyType, NoiseBlend, TransformChannel, TransformSpace, VarType};
use crate::doc::Doc;
use crate::draw::Rect;
use crate::editors::properties::layout::{L, ROW};
use crate::theme as t;
use crate::ui::{MenuItem, NumOpts, Op, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    FCurve,
    Modifiers,
    Drivers,
}

/// The active curve: the active channel, else the first selected curve.
pub fn active_curve(s: &State, d: &Doc) -> Option<(usize, usize, usize)> {
    if let Some(Row::Curve(o, a, c)) = s.active {
        if d.anim.actions.get(a).is_some_and(|x| c < x.curves.len()) {
            return Some((o, a, c));
        }
    }
    for (o, oa) in d.anim.objects.iter().enumerate() {
        if let Some(a) = oa.edit_action() {
            if let Some(c) = d.anim.actions[a].curves.iter().position(|f| f.select) {
                return Some((o, a, c));
            }
        }
    }
    None
}

/// The active driver: the active channel, else the first selected one.
pub fn active_driver(s: &State, d: &Doc) -> Option<(usize, usize)> {
    if let Some(Row::Driver(o, i)) = s.active {
        if d.anim.objects.get(o).is_some_and(|x| i < x.drivers.len()) {
            return Some((o, i));
        }
    }
    for (o, oa) in d.anim.objects.iter().enumerate() {
        if let Some(i) = oa.drivers.iter().position(|x| x.select) {
            return Some((o, i));
        }
    }
    d.anim.objects.iter().enumerate().find(|(_, x)| !x.drivers.is_empty()).map(|(o, _)| (o, 0))
}

fn set_items(target: &str, key: &str, items: &[(&str, &str)]) -> Vec<MenuItem> {
    items.iter().map(|(id, label)| MenuItem::op(label, "", Op::with("anim.prop_set", &format!("{target}\t{key}={id}")))).collect()
}

pub fn draw(s: &mut State, d: &mut Doc, ui: &mut Ui, r: Rect, _rows: &[Row]) {
    ui.p.fill(r, t::PROPERTIES);
    ui.p.fill(Rect::new(r.x, r.y, 1, r.h), 0x222222);
    // Tab strip.
    let tabs: Vec<(Tab, &str)> = if s.drivers() { vec![(Tab::Drivers, "Drivers"), (Tab::Modifiers, "Modifiers")] } else { vec![(Tab::FCurve, "F-Curve"), (Tab::Modifiers, "Modifiers")] };
    if !tabs.iter().any(|x| x.0 == s.side_tab) {
        s.side_tab = tabs[0].0;
    }
    let labels: Vec<&str> = tabs.iter().map(|x| x.1).collect();
    let mut sel = tabs.iter().position(|x| x.0 == s.side_tab).unwrap_or(0);
    let tr = Rect::new(r.x + 8, r.y + 4, r.w - 16, ROW);
    if ui.segmented(tr, &labels, &mut sel) {
        s.side_tab = tabs[sel].0;
    }
    ui.p.push_clip(Rect::new(r.x, r.y + ROW + 8, r.w, r.h - ROW - 8));
    let drivers = s.drivers();
    let curve = if drivers { None } else { active_curve(s, d) };
    let driver = active_driver(s, d);
    let tab = s.side_tab;
    let mut l = L {
        ui: &mut *ui,
        open: &mut s.open,
        x: r.x + 4,
        w: r.w - 8,
        y: r.y + ROW + 10,
        query: String::new(),
        dry: false,
        hits: 0,
        depth: 0,
        inactive: false,
        prefix: format!("graphside/{:?}", s.side_tab),
        undo_for: s.undo_for,
        fields: Vec::new(),
    };
    match tab {
        Tab::FCurve => fcurve_tab(&mut l, d, curve),
        Tab::Modifiers => {
            if drivers {
                l.note("Driver curves have no modifiers here");
            } else {
                modifiers_tab(&mut l, d, curve)
            }
        }
        Tab::Drivers => drivers_tab(&mut l, d, driver),
    }
    let undo_for = l.undo_for;
    s.undo_for = undo_for;
    // A finished drag ends the undo group.
    if ui.st.active.is_none() {
        s.undo_for = None;
    }
    ui.p.pop_clip();
}

fn fcurve_tab(l: &mut L, d: &mut Doc, curve: Option<(usize, usize, usize)>) {
    let Some((_, a, c)) = curve else {
        l.note("No active F-Curve");
        return;
    };
    let target = format!("fcu:{a}:{c}");
    let fc = d.anim.actions[a].curves[c].clone();
    l.panel(d, "fcurve", "F-Curve", true, &mut |l, d| {
        l.info("", &fc.label());
        l.info("Data Path", &fc.path);
        l.info("Array Index", &fc.index.to_string());
        l.info("Group", &fc.group);
        let ext = if fc.extrapolation == anim::Extrapolation::Linear { "Linear" } else { "Constant" };
        let tgt = target.clone();
        l.dropdown_row("Extrapolation", "extrapolation", ext, &|| set_items(&tgt, "extrapolation", &[("CONSTANT", "Constant"), ("LINEAR", "Linear")]));
        let mut mute = fc.mute;
        if l.check("", "Mute", &mut mute) {
            l.commit(d, "Mute F-Curve");
            d.anim.actions[a].curves[c].mute = mute;
            d.anim.dirty = true;
        }
        let mut lock = fc.lock;
        if l.check("", "Lock", &mut lock) {
            l.commit(d, "Lock F-Curve");
            d.anim.actions[a].curves[c].lock = lock;
        }
    });
    let Some(k) = fc.keys.iter().position(|k| k.select || k.select_left || k.select_right) else { return };
    let key = fc.keys[k].clone();
    let kt = format!("key:{a}:{c}:{k}");
    l.panel(d, "activekey", "Active Keyframe", true, &mut |l, d| {
        let tg = kt.clone();
        l.dropdown_row("Interpolation", "interpolation", key.interp.label(), &|| set_items(&tg, "interpolation", &Interp::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
        if !matches!(key.interp, Interp::Constant | Interp::Linear | Interp::Bezier) {
            let tg = kt.clone();
            l.dropdown_row("Easing", "easing", key.easing.label(), &|| set_items(&tg, "easing", &Easing::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
        }
        let tg = kt.clone();
        l.dropdown_row("Key Type", "type", key.key_type.label(), &|| set_items(&tg, "type", &KeyType::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
        let mut frame = key.frame;
        if l.number("Key Frame", "kframe", &mut frame, NumOpts { step: 1.0, ..NumOpts::default() }) {
            l.commit(d, "Move Keyframe");
            move_key(d, a, c, k, Some(frame), None);
        }
        let mut value = key.value;
        if l.number("Value", "kvalue", &mut value, NumOpts { step: 0.1, ..NumOpts::default() }) {
            l.commit(d, "Change Keyframe Value");
            move_key(d, a, c, k, None, Some(value));
        }
        if key.interp == Interp::Bezier {
            l.space(4);
            let tg = kt.clone();
            l.dropdown_row("Left Handle Type", "h1", key.h1.label(), &|| set_items(&tg, "handle_left_type", &HandleType::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
            for (i, (lab, keyname)) in [("Frame", "lx"), ("Value", "ly")].iter().enumerate() {
                let mut v = key.left[i];
                if l.number(lab, keyname, &mut v, NumOpts { step: 0.1, ..NumOpts::default() }) {
                    l.commit(d, "Move Handle");
                    set_handle(d, a, c, k, false, i, v);
                }
            }
            let tg = kt.clone();
            l.dropdown_row("Right Handle Type", "h2", key.h2.label(), &|| set_items(&tg, "handle_right_type", &HandleType::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
            for (i, (lab, keyname)) in [("Frame", "rx"), ("Value", "ry")].iter().enumerate() {
                let mut v = key.right[i];
                if l.number(lab, keyname, &mut v, NumOpts { step: 0.1, ..NumOpts::default() }) {
                    l.commit(d, "Move Handle");
                    set_handle(d, a, c, k, true, i, v);
                }
            }
        }
    });
}

fn move_key(d: &mut Doc, a: usize, c: usize, k: usize, frame: Option<f32>, value: Option<f32>) {
    let fc = &mut d.anim.actions[a].curves[c];
    let Some(kf) = fc.keys.get_mut(k) else { return };
    if let Some(f) = frame {
        let df = f - kf.frame;
        kf.frame = f;
        kf.left[0] += df;
        kf.right[0] += df;
    }
    if let Some(v) = value {
        let dv = v - kf.value;
        kf.value = v;
        kf.left[1] += dv;
        kf.right[1] += dv;
    }
    fc.recalc_handles();
    d.anim.dirty = true;
}

fn set_handle(d: &mut Doc, a: usize, c: usize, k: usize, right: bool, i: usize, v: f32) {
    let fc = &mut d.anim.actions[a].curves[c];
    let Some(kf) = fc.keys.get_mut(k) else { return };
    // Editing a handle by number frees automatic handles, as in Blender.
    if right {
        kf.right[i] = v;
        if matches!(kf.h2, HandleType::Auto | HandleType::AutoClamped) {
            kf.h2 = HandleType::Aligned;
        }
    } else {
        kf.left[i] = v;
        if matches!(kf.h1, HandleType::Auto | HandleType::AutoClamped) {
            kf.h1 = HandleType::Aligned;
        }
    }
    fc.recalc_handles();
    d.anim.dirty = true;
}

fn modifiers_tab(l: &mut L, d: &mut Doc, curve: Option<(usize, usize, usize)>) {
    let Some((_, a, c)) = curve else {
        l.note("No active F-Curve");
        return;
    };
    if l.band(ROW) {
        let (x, w) = l.inner();
        let r = Rect::new(x, l.y, w, ROW);
        if l.ui.dropdown(r, "Add Modifier") {
            l.ui.open_menu(
                r,
                "Add Modifier",
                vec![MenuItem::op("Cycles", "", Op::with("graph.fmodifier_add", "CYCLES:ACTIVE")), MenuItem::op("Noise", "", Op::with("graph.fmodifier_add", "NOISE:ACTIVE"))],
            );
        }
        l.y += ROW + 4;
    }
    let mods = d.anim.actions[a].curves[c].modifiers.clone();
    for (m, md) in mods.iter().enumerate() {
        let target = format!("fmod:{a}:{c}:{m}");
        l.panel_with(
            d,
            &format!("fmod{m}"),
            md.name(),
            true,
            &mut |l, d, hr| {
                let xr = Rect::new(hr.right() - 20, hr.y, 20, hr.h);
                if l.ui.button(xr, "X") {
                    d.queue.push(Op::with("graph.fmodifier_remove", &target));
                }
                let mr = Rect::new(hr.right() - 44, hr.y, 22, hr.h);
                if l.ui.button(mr, if md.muted() { "o" } else { "v" }) {
                    d.queue.push(Op::with("anim.prop_set", &format!("{target}\tmute=TOGGLE")));
                }
            },
            &mut |l, d| match md {
                FModifier::Cycles { before, after, .. } => {
                    let items = |key: &'static str| {
                        let tg = target.clone();
                        move || set_items(&tg, key, &CycleMode::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>())
                    };
                    l.dropdown_row("Before Mode", "before", before.label(), &items("before"));
                    l.dropdown_row("After Mode", "after", after.label(), &items("after"));
                }
                FModifier::Noise { blend, scale, strength, phase, offset, depth, .. } => {
                    let tg = target.clone();
                    l.dropdown_row("Blend Type", "blend", blend.label(), &|| set_items(&tg, "blend", &NoiseBlend::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
                    for (lab, key, v) in [("Scale", "scale", *scale), ("Strength", "strength", *strength), ("Phase", "phase", *phase), ("Offset", "offset", *offset)] {
                        let mut x = v;
                        if l.number(lab, key, &mut x, NumOpts { step: 0.1, ..NumOpts::default() }) {
                            l.commit(d, "Change Noise");
                            anim::props::set(d, &target, key, &x.to_string());
                            d.anim.dirty = true;
                        }
                    }
                    let mut dp = *depth as f32;
                    if l.number("Depth", "depth", &mut dp, NumOpts { step: 1.0, min: 0.0, max: 32.0, int: true, ..NumOpts::default() }) {
                        l.commit(d, "Change Noise");
                        anim::props::set(d, &target, "depth", &(dp as u32).to_string());
                        d.anim.dirty = true;
                    }
                }
            },
        );
    }
}

fn drivers_tab(l: &mut L, d: &mut Doc, driver: Option<(usize, usize)>) {
    let Some((o, i)) = driver else {
        l.note("No active driver");
        l.note("Ctrl+D over a property adds one");
        return;
    };
    let drv = d.anim.objects[o].drivers[i].clone();
    let owner = d.anim.objects[o].object.clone();
    let target = format!("drv:{o}:{i}");
    l.panel(d, "driven", "Driven Property", true, &mut |l, _d| {
        l.info("Object", &owner);
        l.info("Property", &drv.label());
    });
    l.panel(d, "driver", "Driver", true, &mut |l, d| {
        let tg = target.clone();
        l.dropdown_row("Type", "type", drv.kind.label(), &|| set_items(&tg, "type", &DriverType::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
        if drv.kind == DriverType::Scripted {
            let mut e = drv.expression.clone();
            if l.text("Expression", "expr", &mut e) {
                l.commit(d, "Change Expression");
                d.anim.objects[o].drivers[i].expression = e;
                d.anim.dirty = true;
            }
        }
        match &drv.error {
            Some(e) => l.note(&format!("ERROR: {e}")),
            None => l.info("Driver Value", &format!("{:.3}", drv.value)),
        }
        if l.button("Add Input Variable") {
            d.queue.push(Op::with("anim.driver_variable_add", &target));
        }
        for (v, var) in drv.vars.iter().enumerate() {
            let vt = format!("var:{o}:{i}:{v}");
            l.space(4);
            let mut name = var.name.clone();
            if l.text("Variable", &format!("vname{v}"), &mut name) {
                l.commit(d, "Rename Variable");
                anim::props::set(d, &vt, "name", &name);
                d.anim.dirty = true;
            }
            let tg = vt.clone();
            l.dropdown_row("Type", &format!("vtype{v}"), var.kind.label(), &|| set_items(&tg, "type", &VarType::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
            let ntargets = if matches!(var.kind, VarType::RotDiff | VarType::LocDiff) { 2 } else { 1 };
            for ti in 0..ntargets {
                let tgt = &var.targets[ti];
                let pre = if ti == 0 { "t0." } else { "t1." };
                let objs: Vec<String> = d.scene.objects.iter().map(|x| x.name.clone()).collect();
                let tg = vt.clone();
                let label = if ntargets == 2 { format!("Object {}", ti + 1) } else { "Object".to_string() };
                l.dropdown_row(&label, &format!("vobj{v}{ti}"), if tgt.object.is_empty() { "(none)" } else { &tgt.object }, &|| {
                    objs.iter().map(|n| MenuItem::op(n, "", Op::with("anim.prop_set", &format!("{tg}\t{pre}object={n}")))).collect()
                });
                if var.kind != VarType::SingleProp {
                    let mut bone = tgt.bone.clone();
                    if l.text("Bone", &format!("vbone{v}{ti}"), &mut bone) {
                        l.commit(d, "Change Variable");
                        anim::props::set(d, &vt, &format!("{pre}bone"), &bone);
                        d.anim.dirty = true;
                    }
                }
                match var.kind {
                    VarType::SingleProp => {
                        let mut p = tgt.path.clone();
                        if l.text("Path", &format!("vpath{v}"), &mut p) {
                            l.commit(d, "Change Variable");
                            anim::props::set(d, &vt, "path", &p);
                            d.anim.dirty = true;
                        }
                    }
                    VarType::Transforms => {
                        let tg = vt.clone();
                        l.dropdown_row("Type", &format!("vchan{v}"), tgt.channel.label(), &|| set_items(&tg, "channel", &TransformChannel::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
                        let tg = vt.clone();
                        l.dropdown_row("Space", &format!("vspace{v}"), tgt.space.label(), &|| set_items(&tg, "space", &TransformSpace::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
                    }
                    VarType::LocDiff => {
                        let tg = vt.clone();
                        let p = pre.to_string();
                        l.dropdown_row("Space", &format!("vspace{v}{ti}"), tgt.space.label(), &|| set_items(&tg, &format!("{p}space"), &TransformSpace::ALL.iter().map(|x| (x.id(), x.label())).collect::<Vec<_>>()));
                    }
                    VarType::RotDiff => {}
                }
            }
            match &var.error {
                Some(e) => l.note(&format!("ERROR: {e}")),
                None => l.info("Value", &format!("{:.3}", var.value)),
            }
            if l.button("Remove Variable") {
                d.queue.push(Op::with("anim.driver_variable_remove", &vt));
            }
        }
    });
}
