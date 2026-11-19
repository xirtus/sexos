// SPDX-License-Identifier: GPL-3.0-only
//! `anim.prop_set "TARGET\tkey=value"`: the enum and text properties the
//! animation editors' sidebars change (driver type and variables, F-curve
//! modifiers, the active key, NLA strips), plus adding and removing driver
//! variables and modifiers. Targets: `drv:o:d`, `var:o:d:v`, `fmod:a:c:m`,
//! `key:a:c:k`, `fcu:a:c`, `strip:o:t:s`.
use super::*;

fn nums(s: &str) -> Vec<usize> {
    s.split(':').skip(1).filter_map(|x| x.parse().ok()).collect()
}

pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    match op.id.as_str() {
        "anim.prop_set" => {
            let Some((target, kv)) = arg.split_once('\t') else { return true };
            let Some((key, value)) = kv.split_once('=') else { return true };
            doc.push_undo("Change Property");
            set(doc, target, key, value);
            doc.anim.dirty = true;
        }
        "anim.driver_variable_add" => {
            let n = nums(arg);
            let [o, d] = n[..] else { return true };
            doc.push_undo("Add Variable");
            if let Some(drv) = doc.anim.objects.get_mut(o).and_then(|x| x.drivers.get_mut(d)) {
                let name = sex_3d::scene::unique_name("var", |n| drv.vars.iter().any(|v| v.name == n)).replace('.', "_");
                let mut v = DriverVar::new(&name, VarType::SingleProp);
                v.targets[0].object = doc.anim.objects[o].object.clone();
                doc.anim.objects[o].drivers[d].vars.push(v);
            }
            doc.anim.dirty = true;
        }
        "anim.driver_variable_remove" => {
            let n = nums(arg);
            let [o, d, v] = n[..] else { return true };
            doc.push_undo("Remove Variable");
            if let Some(drv) = doc.anim.objects.get_mut(o).and_then(|x| x.drivers.get_mut(d)) {
                if v < drv.vars.len() {
                    drv.vars.remove(v);
                }
            }
            doc.anim.dirty = true;
        }
        "graph.fmodifier_remove" => {
            let n = nums(arg);
            let [a, c, m] = n[..] else { return true };
            doc.push_undo("Remove F-Curve Modifier");
            if let Some(fc) = doc.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)) {
                if m < fc.modifiers.len() {
                    fc.modifiers.remove(m);
                }
            }
            doc.anim.dirty = true;
        }
        _ => return false,
    }
    true
}

/// Set `key` of `target` from its text value.
pub fn set(doc: &mut Doc, target: &str, key: &str, v: &str) {
    let n = nums(target);
    let fl = |cur: bool| match v {
        "TOGGLE" => !cur,
        x => x == "1" || x == "true",
    };
    let num = |cur: f32| v.parse::<f32>().unwrap_or(cur);
    match target.split(':').next().unwrap_or("") {
        "drv" => {
            let [o, d] = n[..] else { return };
            let Some(drv) = doc.anim.objects.get_mut(o).and_then(|x| x.drivers.get_mut(d)) else { return };
            match key {
                "type" => {
                    if let Some(t) = DriverType::from_arg(v) {
                        drv.kind = t;
                    }
                }
                "expression" => drv.expression = v.to_string(),
                "mute" => drv.mute = fl(drv.mute),
                _ => {}
            }
        }
        "var" => {
            let [o, d, i] = n[..] else { return };
            let Some(var) = doc.anim.objects.get_mut(o).and_then(|x| x.drivers.get_mut(d)).and_then(|x| x.vars.get_mut(i)) else { return };
            let (t, k) = match key.strip_prefix("t1.") {
                Some(k) => (1, k),
                None => (0, key.strip_prefix("t0.").unwrap_or(key)),
            };
            match k {
                "name" => var.name = v.to_string(),
                "type" => {
                    if let Some(x) = VarType::from_arg(v) {
                        var.kind = x;
                    }
                }
                "object" => var.targets[t].object = v.to_string(),
                "path" => var.targets[t].path = v.to_string(),
                "bone" => var.targets[t].bone = v.to_string(),
                "channel" => {
                    if let Some(x) = TransformChannel::from_arg(v) {
                        var.targets[t].channel = x;
                    }
                }
                "space" => {
                    if let Some(x) = TransformSpace::from_arg(v) {
                        var.targets[t].space = x;
                    }
                }
                _ => {}
            }
        }
        "fmod" => {
            let [a, c, m] = n[..] else { return };
            let Some(md) = doc.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)).and_then(|x| x.modifiers.get_mut(m)) else { return };
            match md {
                FModifier::Cycles { before, after, mute } => match key {
                    "before" => *before = CycleMode::from_arg(v).unwrap_or(*before),
                    "after" => *after = CycleMode::from_arg(v).unwrap_or(*after),
                    "mute" => *mute = fl(*mute),
                    _ => {}
                },
                FModifier::Noise { blend, scale, strength, phase, offset, depth, mute } => match key {
                    "blend" => *blend = NoiseBlend::from_arg(v).unwrap_or(*blend),
                    "scale" => *scale = num(*scale),
                    "strength" => *strength = num(*strength),
                    "phase" => *phase = num(*phase),
                    "offset" => *offset = num(*offset),
                    "depth" => *depth = v.parse().unwrap_or(*depth),
                    "mute" => *mute = fl(*mute),
                    _ => {}
                },
            }
        }
        "key" => {
            let [a, c, k] = n[..] else { return };
            let Some(fc) = doc.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)) else { return };
            let Some(kf) = fc.keys.get_mut(k) else { return };
            match key {
                "interpolation" => kf.interp = Interp::from_arg(v).unwrap_or(kf.interp),
                "easing" => kf.easing = Easing::from_arg(v).unwrap_or(kf.easing),
                "handle_left_type" => kf.h1 = HandleType::from_arg(v).unwrap_or(kf.h1),
                "handle_right_type" => kf.h2 = HandleType::from_arg(v).unwrap_or(kf.h2),
                "type" => kf.key_type = KeyType::from_arg(v).unwrap_or(kf.key_type),
                _ => {}
            }
            fc.recalc_handles();
        }
        "fcu" => {
            let [a, c] = n[..] else { return };
            let Some(fc) = doc.anim.actions.get_mut(a).and_then(|x| x.curves.get_mut(c)) else { return };
            match key {
                "mute" => fc.mute = fl(fc.mute),
                "hide" => fc.hide = fl(fc.hide),
                "lock" => fc.lock = fl(fc.lock),
                "extrapolation" => fc.extrapolation = if v == "LINEAR" { Extrapolation::Linear } else { Extrapolation::Constant },
                _ => {}
            }
        }
        "strip" => {
            let [o, t, s] = n[..] else { return };
            let Some(st) = doc.anim.objects.get_mut(o).and_then(|x| x.tracks.get_mut(t)).and_then(|x| x.strips.get_mut(s)) else { return };
            match key {
                "blend_type" => st.blend = Blend::from_arg(v).unwrap_or(st.blend),
                "extrapolation" => st.extrapolation = StripExtrap::from_arg(v).unwrap_or(st.extrapolation),
                "mute" => st.mute = fl(st.mute),
                "use_reverse" => st.reversed = fl(st.reversed),
                "use_animated_influence" => st.use_influence = fl(st.use_influence),
                "action" => {
                    if let Some(ai) = doc.anim.actions.iter().position(|a| a.name == v) {
                        st.action = ai;
                    }
                }
                _ => {}
            }
        }
        "adata" => {
            let [o] = n[..] else { return };
            let Some(oa) = doc.anim.objects.get_mut(o) else { return };
            match key {
                "blend_type" => oa.blend = Blend::from_arg(v).unwrap_or(oa.blend),
                "influence" => oa.influence = num(oa.influence),
                _ => {}
            }
        }
        _ => {}
    }
}
