// SPDX-License-Identifier: GPL-3.0-only
//! `#@` lines for animation (actions, F-curves and their modifiers, NLA,
//! drivers) and for the rigging UI state, plus the old `#@key` lines of
//! files written before generic data paths.
use super::{esc, unesc};
use crate::anim::{self, Action, Blend, CycleMode, Driver, DriverTarget, DriverType, DriverVar, Easing, Extrapolation, FCurve, FModifier, HandleType, Interp, KeyType, Keyframe, NlaStrip, NlaTrack, NoiseBlend, ObjectAnim, StripExtrap, TransformChannel, TransformSpace, VarType};
use crate::doc::Doc;
use crate::rig::{ArmUi, BoneDisplay, BoneSel, WpBlend, WpBrush};
use std::fmt::Write;

fn b(v: bool) -> u8 {
    v as u8
}

pub fn save(doc: &Doc, out: &mut String) {
    let a = &doc.anim;
    let _ = writeln!(out, "#@anim {} {} {} {} {} {}", b(a.auto_key), b(a.reverse), a.interp.id(), a.handle.id(), a.key_type.id(), a.keying_set.as_deref().unwrap_or("-"));
    for act in &a.actions {
        let _ = writeln!(out, "#@action {}\t{}", esc(&act.name), b(act.fake_user));
        for c in &act.curves {
            let ex = match c.extrapolation {
                Extrapolation::Constant => "CONSTANT",
                Extrapolation::Linear => "LINEAR",
            };
            let _ = writeln!(out, "#@fcurve {}\t{}\t{}\t{}\t{} {} {} {}", esc(&c.path), c.index, esc(&c.group), ex, b(c.mute), b(c.hide), b(c.lock), b(c.select));
            for m in &c.modifiers {
                match m {
                    FModifier::Cycles { before, after, mute } => {
                        let _ = writeln!(out, "#@fmod CYCLES {} {} {}", before.id(), after.id(), b(*mute));
                    }
                    FModifier::Noise { blend, scale, strength, phase, offset, depth, mute } => {
                        let _ = writeln!(out, "#@fmod NOISE {} {scale} {strength} {phase} {offset} {depth} {}", blend.id(), b(*mute));
                    }
                }
            }
            for k in &c.keys {
                let _ = writeln!(
                    out,
                    "#@kf {} {} {} {} {} {} {} {} {} {} {}",
                    k.frame,
                    k.value,
                    k.interp.id(),
                    k.easing.id(),
                    k.left[0],
                    k.left[1],
                    k.right[0],
                    k.right[1],
                    k.h1.id(),
                    k.h2.id(),
                    k.key_type.id()
                );
            }
        }
    }
    for o in &a.objects {
        let tweak = o.tweak.map_or("-1 -1".to_string(), |(t, s)| format!("{t} {s}"));
        let act = o.action.map_or(-1, |x| x as i64);
        let _ = writeln!(out, "#@adata {}\t{act} {} {} {tweak}", esc(&o.object), o.blend.id(), o.influence);
        for t in &o.tracks {
            let _ = writeln!(out, "#@track {}\t{}\t{} {} {}", esc(&o.object), esc(&t.name), b(t.mute), b(t.solo), b(t.lock));
            for s in &t.strips {
                let _ = writeln!(
                    out,
                    "#@strip {}\t{}\t{} {} {} {} {} {} {} {} {} {} {} {} {} {} {} {}",
                    esc(&o.object),
                    esc(&s.name),
                    s.action,
                    s.frame_start,
                    s.frame_end,
                    s.action_start,
                    s.action_end,
                    s.scale,
                    s.repeat,
                    s.blend_in,
                    s.blend_out,
                    s.extrapolation.id(),
                    s.blend.id(),
                    s.influence,
                    b(s.use_influence),
                    b(s.mute),
                    b(s.reversed),
                    b(s.select)
                );
            }
        }
        for d in &o.drivers {
            let _ = writeln!(out, "#@driver {}\t{}\t{}\t{}\t{}\t{}", esc(&o.object), esc(&d.path), d.index, d.kind.id(), esc(&d.expression), b(d.mute));
            for v in &d.vars {
                let t = |t: &DriverTarget| format!("{}\t{}\t{}\t{}\t{}", esc(&t.object), esc(&t.path), esc(&t.bone), t.channel.id(), t.space.id());
                let _ = writeln!(out, "#@dvar {}\t{}\t{}\t{}", esc(&v.name), v.kind.id(), t(&v.targets[0]), t(&v.targets[1]));
            }
        }
    }
    // Rigging UI state.
    let r = &doc.rig;
    let wp = &r.wp;
    let _ = writeln!(out, "#@rig {} {} {} {} {} {} {} {} {}", b(r.auto_ik), wp.brush.label(), wp.blend.label(), wp.weight, wp.strength, wp.radius, b(wp.auto_normalize), b(wp.x_mirror), wp.vg_weight);
    for (name, ui) in &r.arms {
        let _ = writeln!(out, "#@arm {}\t{} {} {} {} {}\t{}", esc(name), ui.display.id(), b(ui.show_names), b(ui.show_axes), b(ui.show_in_front), ui.active_collection, esc(ui.active.as_deref().unwrap_or("")));
        for (bone, s) in &ui.sel {
            let _ = writeln!(out, "#@bsel {}\t{}\t{}{}{}", esc(name), esc(bone), b(s.body), b(s.head), b(s.tail));
        }
        for h in &ui.hidden {
            let _ = writeln!(out, "#@bhide {}\t{}", esc(name), esc(h));
        }
        for h in &ui.hidden_collections {
            let _ = writeln!(out, "#@bchide {}\t{}", esc(name), esc(h));
        }
    }
    for (o, g) in &r.active_group {
        let _ = writeln!(out, "#@vgactive {}\t{g}", esc(o));
    }
    for (o, g) in &r.group_lock {
        let _ = writeln!(out, "#@vglock {}\t{}", esc(o), esc(g));
    }
    for (o, k) in &r.active_shape {
        let _ = writeln!(out, "#@skactive {}\t{k}", esc(o));
    }
    for o in &r.shape_lock {
        let _ = writeln!(out, "#@sklock {}", esc(o));
    }
    for o in &r.shape_edit_mode {
        let _ = writeln!(out, "#@skedit {}", esc(o));
    }
    for (o, s) in &r.lattice_sel {
        let _ = writeln!(out, "#@latsel {}\t{}", esc(o), s.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
    }
    for ai in 0..doc.scene.armatures.len() {
        if doc.scene.armatures[ai].rest_position {
            let _ = writeln!(out, "#@restpos {}", esc(&doc.scene.armature_names[ai]));
        }
    }
}

fn f(s: Option<&str>) -> f32 {
    s.and_then(|s| s.parse().ok()).unwrap_or(0.0)
}

/// Loader state across lines.
#[derive(Default)]
pub struct Loader {
    action: Option<usize>,
    curve: Option<usize>,
    driver: Option<(usize, usize)>,
}

/// One `#@` line; returns false when the tag is not ours.
pub fn load_line(doc: &mut Doc, st: &mut Loader, tag: &str, body: &str) -> bool {
    match tag {
        "anim" => {
            let mut w = body.split_whitespace();
            doc.anim.auto_key = w.next() == Some("1");
            doc.anim.reverse = w.next() == Some("1");
            if let Some(i) = w.next().and_then(Interp::from_arg) {
                doc.anim.interp = i;
            }
            if let Some(h) = w.next().and_then(HandleType::from_arg) {
                doc.anim.handle = h;
            }
            if let Some(k) = w.next().and_then(KeyType::from_arg) {
                doc.anim.key_type = k;
            }
            doc.anim.keying_set = w.next().filter(|k| *k != "-").map(|k| k.to_string());
        }
        // Files from before data paths: "object path index frame value interp lx ly rx ry".
        "key" => {
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 10 {
                return true;
            }
            let (Ok(index), Some(interp)) = (p[2].parse::<usize>(), Interp::from_arg(p[5])) else { return true };
            let n: Vec<f32> = [p[3], p[4], p[6], p[7], p[8], p[9]].iter().map(|s| s.parse().unwrap_or(0.0)).collect();
            let obj = unesc(p[0]);
            let a = doc.anim.ensure_action(&obj);
            let c = doc.anim.actions[a].curve_mut(p[1], index);
            let mut k = Keyframe::new(n[0], n[1], interp);
            k.left = [n[2], n[3]];
            k.right = [n[4], n[5]];
            k.h1 = HandleType::Free;
            k.h2 = HandleType::Free;
            c.keys.push(k);
            c.sort();
        }
        "action" => {
            let (name, fake) = body.split_once('\t').unwrap_or((body, "0"));
            let mut act = Action::new(&unesc(name));
            act.fake_user = fake == "1";
            doc.anim.actions.push(act);
            st.action = Some(doc.anim.actions.len() - 1);
            st.curve = None;
        }
        "fcurve" => {
            let Some(a) = st.action else { return true };
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 5 {
                return true;
            }
            let mut c = FCurve::new(&unesc(p[0]), p[1].parse().unwrap_or(0));
            c.group = unesc(p[2]);
            c.extrapolation = if p[3] == "LINEAR" { Extrapolation::Linear } else { Extrapolation::Constant };
            let fl: Vec<bool> = p[4].split_whitespace().map(|x| x == "1").collect();
            c.mute = fl.first().copied().unwrap_or(false);
            c.hide = fl.get(1).copied().unwrap_or(false);
            c.lock = fl.get(2).copied().unwrap_or(false);
            c.select = fl.get(3).copied().unwrap_or(false);
            doc.anim.actions[a].curves.push(c);
            st.curve = Some(doc.anim.actions[a].curves.len() - 1);
        }
        "fmod" => {
            let (Some(a), Some(c)) = (st.action, st.curve) else { return true };
            let w: Vec<&str> = body.split_whitespace().collect();
            let m = match w.first().copied() {
                Some("CYCLES") => FModifier::Cycles {
                    before: w.get(1).and_then(|s| CycleMode::from_arg(s)).unwrap_or(CycleMode::Repeat),
                    after: w.get(2).and_then(|s| CycleMode::from_arg(s)).unwrap_or(CycleMode::Repeat),
                    mute: w.get(3) == Some(&"1"),
                },
                Some("NOISE") => FModifier::Noise {
                    blend: w.get(1).and_then(|s| NoiseBlend::from_arg(s)).unwrap_or(NoiseBlend::Replace),
                    scale: f(w.get(2).copied()),
                    strength: f(w.get(3).copied()),
                    phase: f(w.get(4).copied()),
                    offset: f(w.get(5).copied()),
                    depth: w.get(6).and_then(|s| s.parse().ok()).unwrap_or(0),
                    mute: w.get(7) == Some(&"1"),
                },
                _ => return true,
            };
            doc.anim.actions[a].curves[c].modifiers.push(m);
        }
        "kf" => {
            let (Some(a), Some(c)) = (st.action, st.curve) else { return true };
            let w: Vec<&str> = body.split_whitespace().collect();
            if w.len() < 11 {
                return true;
            }
            let mut k = Keyframe::new(f(Some(w[0])), f(Some(w[1])), Interp::from_arg(w[2]).unwrap_or(Interp::Bezier));
            k.easing = Easing::from_arg(w[3]).unwrap_or(k.easing);
            k.left = [f(Some(w[4])), f(Some(w[5]))];
            k.right = [f(Some(w[6])), f(Some(w[7]))];
            k.h1 = HandleType::from_arg(w[8]).unwrap_or(k.h1);
            k.h2 = HandleType::from_arg(w[9]).unwrap_or(k.h2);
            k.key_type = KeyType::from_arg(w[10]).unwrap_or(k.key_type);
            doc.anim.actions[a].curves[c].keys.push(k);
        }
        "adata" => {
            let Some((name, rest)) = body.split_once('\t') else { return true };
            let w: Vec<&str> = rest.split_whitespace().collect();
            let o = doc.anim.entry(&unesc(name));
            let oa: &mut ObjectAnim = &mut doc.anim.objects[o];
            let act: i64 = w.first().and_then(|s| s.parse().ok()).unwrap_or(-1);
            oa.action = (act >= 0).then_some(act as usize);
            oa.blend = w.get(1).and_then(|s| Blend::from_arg(s)).unwrap_or(Blend::Replace);
            oa.influence = w.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let t: i64 = w.get(3).and_then(|s| s.parse().ok()).unwrap_or(-1);
            let s: i64 = w.get(4).and_then(|s| s.parse().ok()).unwrap_or(-1);
            oa.tweak = (t >= 0 && s >= 0).then_some((t as usize, s as usize));
        }
        "track" => {
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 3 {
                return true;
            }
            let o = doc.anim.entry(&unesc(p[0]));
            let mut t = NlaTrack::new(&unesc(p[1]));
            let fl: Vec<bool> = p[2].split_whitespace().map(|x| x == "1").collect();
            t.mute = fl.first().copied().unwrap_or(false);
            t.solo = fl.get(1).copied().unwrap_or(false);
            t.lock = fl.get(2).copied().unwrap_or(false);
            doc.anim.objects[o].tracks.push(t);
        }
        "strip" => {
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 3 {
                return true;
            }
            let o = doc.anim.entry(&unesc(p[0]));
            let w: Vec<&str> = p[2].split_whitespace().collect();
            if w.len() < 16 {
                return true;
            }
            let ai: usize = w[0].parse().unwrap_or(0);
            let Some(act) = doc.anim.actions.get(ai) else { return true };
            let mut s = NlaStrip::new(&unesc(p[1]), ai, act, f(Some(w[1])));
            s.frame_start = f(Some(w[1]));
            s.frame_end = f(Some(w[2]));
            s.action_start = f(Some(w[3]));
            s.action_end = f(Some(w[4]));
            s.scale = f(Some(w[5]));
            s.repeat = f(Some(w[6]));
            s.blend_in = f(Some(w[7]));
            s.blend_out = f(Some(w[8]));
            s.extrapolation = StripExtrap::from_arg(w[9]).unwrap_or(StripExtrap::Hold);
            s.blend = Blend::from_arg(w[10]).unwrap_or(Blend::Replace);
            s.influence = f(Some(w[11]));
            s.use_influence = w[12] == "1";
            s.mute = w[13] == "1";
            s.reversed = w[14] == "1";
            s.select = w[15] == "1";
            if let Some(t) = doc.anim.objects[o].tracks.last_mut() {
                t.strips.push(s);
            }
        }
        "driver" => {
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 6 {
                return true;
            }
            let o = doc.anim.entry(&unesc(p[0]));
            let mut d = Driver::new(&unesc(p[1]), p[2].parse().unwrap_or(0));
            d.kind = DriverType::from_arg(p[3]).unwrap_or(DriverType::Scripted);
            d.expression = unesc(p[4]);
            d.mute = p[5] == "1";
            d.vars.clear();
            doc.anim.objects[o].drivers.push(d);
            st.driver = Some((o, doc.anim.objects[o].drivers.len() - 1));
        }
        "dvar" => {
            let Some((o, di)) = st.driver else { return true };
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 12 {
                return true;
            }
            let mut v = DriverVar::new(&unesc(p[0]), VarType::from_arg(p[1]).unwrap_or(VarType::SingleProp));
            for (i, t) in v.targets.iter_mut().enumerate() {
                let q = &p[2 + i * 5..7 + i * 5];
                t.object = unesc(q[0]);
                t.path = unesc(q[1]);
                t.bone = unesc(q[2]);
                t.channel = TransformChannel::from_arg(q[3]).unwrap_or(TransformChannel::LocX);
                t.space = TransformSpace::from_arg(q[4]).unwrap_or(TransformSpace::World);
            }
            if let Some(d) = doc.anim.objects.get_mut(o).and_then(|x| x.drivers.get_mut(di)) {
                d.vars.push(v);
            }
        }
        "rig" => {
            let w: Vec<&str> = body.split_whitespace().collect();
            let r = &mut doc.rig;
            r.auto_ik = w.first() == Some(&"1");
            if let Some(x) = w.get(1).and_then(|s| WpBrush::from_label(s)) {
                r.wp.brush = x;
            }
            if let Some(x) = w.get(2).and_then(|s| WpBlend::from_label(s)) {
                r.wp.blend = x;
            }
            r.wp.weight = w.get(3).and_then(|s| s.parse().ok()).unwrap_or(r.wp.weight);
            r.wp.strength = w.get(4).and_then(|s| s.parse().ok()).unwrap_or(r.wp.strength);
            r.wp.radius = w.get(5).and_then(|s| s.parse().ok()).unwrap_or(r.wp.radius);
            r.wp.auto_normalize = w.get(6) == Some(&"1");
            r.wp.x_mirror = w.get(7) == Some(&"1");
            r.wp.vg_weight = w.get(8).and_then(|s| s.parse().ok()).unwrap_or(r.wp.vg_weight);
        }
        "arm" => {
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 2 {
                return true;
            }
            let w: Vec<&str> = p[1].split_whitespace().collect();
            let mut ui = ArmUi { display: w.first().and_then(|s| BoneDisplay::from_id(s)).unwrap_or_default(), ..Default::default() };
            ui.show_names = w.get(1) == Some(&"1");
            ui.show_axes = w.get(2) == Some(&"1");
            ui.show_in_front = w.get(3) == Some(&"1");
            ui.active_collection = w.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            ui.active = p.get(2).map(|s| unesc(s)).filter(|s| !s.is_empty());
            doc.rig.arms.insert(unesc(p[0]), ui);
        }
        "bsel" | "bhide" | "bchide" => {
            let p: Vec<&str> = body.split('\t').collect();
            if p.len() < 2 {
                return true;
            }
            let ui = doc.rig.arm_mut(&unesc(p[0]));
            let bone = unesc(p[1]);
            match tag {
                "bsel" => {
                    let c: Vec<bool> = p.get(2).unwrap_or(&"000").chars().map(|c| c == '1').collect();
                    ui.set(&bone, BoneSel { body: c.first().copied().unwrap_or(false), head: c.get(1).copied().unwrap_or(false), tail: c.get(2).copied().unwrap_or(false) });
                }
                "bhide" => {
                    ui.hidden.insert(bone);
                }
                _ => {
                    ui.hidden_collections.insert(bone);
                }
            }
        }
        "vgactive" | "skactive" => {
            let Some((o, v)) = body.split_once('\t') else { return true };
            let v: usize = v.parse().unwrap_or(0);
            let m = if tag == "vgactive" { &mut doc.rig.active_group } else { &mut doc.rig.active_shape };
            m.insert(unesc(o), v);
        }
        "vglock" => {
            if let Some((o, g)) = body.split_once('\t') {
                doc.rig.group_lock.insert((unesc(o), unesc(g)));
            }
        }
        "sklock" => {
            doc.rig.shape_lock.insert(unesc(body));
        }
        "skedit" => {
            doc.rig.shape_edit_mode.insert(unesc(body));
        }
        "latsel" => {
            if let Some((o, s)) = body.split_once('\t') {
                doc.rig.lattice_sel.insert(unesc(o), s.split(',').filter_map(|x| x.parse().ok()).collect());
            }
        }
        "restpos" => {
            let n = unesc(body);
            if let Some(i) = doc.scene.armature_names.iter().position(|x| *x == n) {
                doc.scene.armatures[i].rest_position = true;
            }
        }
        _ => return false,
    }
    true
}

/// After loading: handles of old keys, sorted keys, empty data dropped.
pub fn finish(doc: &mut Doc) {
    for act in &mut doc.anim.actions {
        for c in &mut act.curves {
            c.sort();
        }
    }
    let _ = anim::KEYING_SETS;
}
