// SPDX-License-Identifier: GPL-3.0-only
//! polyops commands for Grease Pencil data and objects and for the Video
//! Sequencer. Settings options are the fields' [`crate::props::Visit`]
//! keys, so every setting the Properties editor shows is in the file.
//!
//! * `gp_new NAME` creates an empty Grease Pencil data-block (no layers or
//!   materials); `gp_set DATA active_layer= active_material=
//!   active_vertex_group= stroke_depth_order=2d_layers|3d_location onion=
//!   onion_mode= onion_opacity= ghost_before= ghost_after= ...`.
//! * `gp_layer DATA NAME group=INDEX|none blend= opacity= hide= lock=
//!   use_lights= tint_color= tint_factor= radius_offset= location= rotation=
//!   scale= parent= parent_bone= pass_index= use_masks= masks=`, creating the
//!   layer when missing; `gp_group DATA NAME parent=INDEX|none hide= lock=
//!   expanded= color_tag=`; `gp_vgroup DATA NAME`.
//! * `gp_material DATA NAME show_stroke= mode=line|dots|box stroke_style=
//!   color=r,g,b,a show_fill= fill_style= fill_color= gradient_type=
//!   mix_color= mix_factor= texture_scale= texture_offset= texture_angle=
//!   use_stroke_holdout= use_fill_holdout= hide= lock= pass_index=`.
//! * `gp_frame DATA LAYER FRAME key_type=keyframe|breakdown|moving_hold|extreme|jitter|generated`
//!   adds an empty keyframe; `gp_stroke DATA LAYER FRAME points=x,y,z,...
//!   radius= opacity= color=r,g,b,a,... rotation= weights=point,group,weight,...
//!   cyclic= material= fill_color= fill_opacity= softness= start_cap=round|flat
//!   end_cap= uv_offset= uv_scale= uv_rotation= init_time=` appends a stroke
//!   (per-point lists take one value for every point; `b64:` lists work).
//! * `add_gpencil NAME data=DATA` plus the placement options of `add_object`.
//! * `gp_modifier_add OBJ TYPE name= ...` (types as `GpModifier::new`) and
//!   `gp_effect_add OBJ TYPE name= ...` (visual effects).
//! * `seq_set use_sequencer= active=STRIP`, `seq_channel N name= lock= mute=`,
//!   `seq_strip NAME type=color|image|movie|sound|scene|text|adjustment|meta|EFFECT
//!   meta=PARENT select= ...` and `seq_modifier_add STRIP TYPE name= ...`.
use super::*;
use crate::grease_pencil::effects::GpEffect;
use crate::grease_pencil::modifiers::GpModifier;
use crate::grease_pencil::{CapType, GpLayer, GpMaterial, GpPoint, GpStroke, GreasePencil, KeyframeType, LayerGroup};
use crate::props::{apply_pairs, to_pairs, Visit};
use crate::sequencer::{Effect, Sequencer, Strip, StripKind, StripModifier};

/// Commands handled here.
pub(super) const COMMANDS: &[&str] = &[
    "gp_new",
    "gp_set",
    "gp_layer",
    "gp_group",
    "gp_vgroup",
    "gp_material",
    "gp_frame",
    "gp_stroke",
    "add_gpencil",
    "gp_modifier_add",
    "gp_effect_add",
    "seq_set",
    "seq_channel",
    "seq_strip",
    "seq_modifier_add",
];

/// Feed the command's unused options to a visitor; options the visitor
/// does not know stay unused (so `done` reports them).
fn visit_opts(cmd: &mut Cmd, f: impl FnOnce(&mut dyn Visit)) -> Res<()> {
    let pairs: Vec<(String, String)> = cmd.opts.iter().filter(|o| !o.2).map(|o| (o.0.clone(), o.1.text.clone())).collect();
    let rest = apply_pairs(pairs, f)?;
    for o in cmd.opts.iter_mut() {
        if !o.2 && !rest.iter().any(|(k, _)| *k == o.0) {
            o.2 = true;
        }
    }
    Ok(())
}

/// A visitor's fields as ` key=value` options.
fn opts_of(f: impl FnOnce(&mut dyn Visit)) -> String {
    to_pairs(f).into_iter().map(|(k, v)| format!(" {k}={}", quote(&v))).collect()
}

fn index_opt(cmd: &mut Cmd, key: &str) -> Res<Option<Option<usize>>> {
    Ok(match cmd.opt(key) {
        Some(t) if t.is_word("none") => Some(None),
        Some(t) => Some(Some(uint(&t.text).map_err(|e| format!("{key}: {e}"))? as usize)),
        None => None,
    })
}

fn cap_of(s: &str) -> Res<CapType> {
    match s.to_ascii_lowercase().as_str() {
        "round" => Ok(CapType::Round),
        "flat" => Ok(CapType::Flat),
        _ => Err(format!("cap: '{s}' is not round or flat")),
    }
}

fn cap_name(c: CapType) -> &'static str {
    match c {
        CapType::Round => "round",
        CapType::Flat => "flat",
    }
}

/// Per-point values: one for all, or `n * width`.
fn per_point(v: Option<Vec<f32>>, n: usize, width: usize, key: &str) -> Res<Option<Vec<f32>>> {
    match v {
        None => Ok(None),
        Some(v) if v.len() == width => Ok(Some(v.iter().copied().cycle().take(n * width).collect())),
        Some(v) if v.len() == n * width => Ok(Some(v)),
        Some(v) => Err(format!("{key}: {} values for {n} points", v.len())),
    }
}

fn find_strip<'a>(strips: &'a mut Vec<Strip>, name: &str) -> Option<&'a mut Strip> {
    for s in strips.iter_mut() {
        if s.name == name {
            return Some(s);
        }
        if let StripKind::Meta { strips: inner } = &mut s.kind {
            if let Some(x) = find_strip(inner, name) {
                return Some(x);
            }
        }
    }
    None
}

impl Session {
    fn gp_data(&self, scene: &Scene, t: &Token) -> Res<usize> {
        if let Ok(o) = self.object(scene, t) {
            if let ObjectData::GreasePencil(g) = scene.objects[o].data {
                return Ok(g);
            }
        }
        scene.grease_pencil_names.iter().position(|n| *n == t.text).ok_or_else(|| format!("no grease pencil named '{}'", t.text))
    }

    fn gp_layer_of(&self, scene: &Scene, cmd: &Cmd) -> Res<(usize, usize)> {
        let g = self.gp_data(scene, cmd.arg(0, "a grease pencil")?)?;
        let name = &cmd.arg(1, "a layer name")?.text;
        let l = scene.grease_pencils[g].find_layer(name).ok_or_else(|| format!("no layer named '{name}'"))?;
        Ok((g, l))
    }

    pub(super) fn gp_command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        match cmd.name.as_str() {
            "gp_new" => {
                cmd.max_args(1)?;
                let name = cmd.arg(0, "a name")?.text.clone();
                let i = scene.add_grease_pencil(&name, GreasePencil::default());
                Ok(Some(quote(&scene.grease_pencil_names[i])))
            }
            "gp_set" => {
                cmd.max_args(1)?;
                let g = self.gp_data(scene, cmd.arg(0, "a grease pencil")?)?;
                let gp = &mut scene.grease_pencils[g];
                gp.active_layer = cmd.u("active_layer")?.map_or(gp.active_layer, |v| v as usize);
                gp.active_material = cmd.u("active_material")?.map_or(gp.active_material, |v| v as usize);
                gp.active_vertex_group = cmd.u("active_vertex_group")?.map_or(gp.active_vertex_group, |v| v as usize);
                visit_opts(cmd, |v| gp.visit(v))?;
                Ok(None)
            }
            "gp_layer" => {
                cmd.max_args(2)?;
                let g = self.gp_data(scene, cmd.arg(0, "a grease pencil")?)?;
                let name = cmd.arg(1, "a layer name")?.text.clone();
                let gp = &mut scene.grease_pencils[g];
                let li = match gp.find_layer(&name) {
                    Some(i) => i,
                    None => {
                        gp.layers.push(GpLayer::new(&name));
                        gp.layers.len() - 1
                    }
                };
                if let Some(grp) = index_opt(cmd, "group")? {
                    gp.layers[li].group = grp;
                }
                visit_opts(cmd, |v| gp.layers[li].visit(v))?;
                Ok(None)
            }
            "gp_group" => {
                cmd.max_args(2)?;
                let g = self.gp_data(scene, cmd.arg(0, "a grease pencil")?)?;
                let name = cmd.arg(1, "a group name")?.text.clone();
                let gp = &mut scene.grease_pencils[g];
                let gi = match gp.groups.iter().position(|x| x.name == name) {
                    Some(i) => i,
                    None => {
                        gp.groups.push(LayerGroup { name, parent: None, hide: false, lock: false, expanded: true, color_tag: 0 });
                        gp.groups.len() - 1
                    }
                };
                if let Some(p) = index_opt(cmd, "parent")? {
                    gp.groups[gi].parent = p;
                }
                visit_opts(cmd, |v| gp.groups[gi].visit(v))?;
                Ok(None)
            }
            "gp_vgroup" => {
                cmd.max_args(2)?;
                let g = self.gp_data(scene, cmd.arg(0, "a grease pencil")?)?;
                let name = cmd.arg(1, "a group name")?.text.clone();
                let gp = &mut scene.grease_pencils[g];
                if !gp.vertex_groups.contains(&name) {
                    gp.vertex_groups.push(name);
                }
                Ok(None)
            }
            "gp_material" => {
                cmd.max_args(2)?;
                let g = self.gp_data(scene, cmd.arg(0, "a grease pencil")?)?;
                let name = cmd.arg(1, "a material name")?.text.clone();
                let gp = &mut scene.grease_pencils[g];
                let mi = gp.add_material(GpMaterial::new(&name, [0.0, 0.0, 0.0, 1.0]));
                visit_opts(cmd, |v| gp.materials[mi].visit(v))?;
                Ok(None)
            }
            "gp_frame" => {
                cmd.max_args(3)?;
                let (g, l) = self.gp_layer_of(scene, cmd)?;
                let frame = num(&cmd.arg(2, "a frame")?.text)? as i32;
                let key = match cmd.opt("key_type") {
                    Some(t) => KeyframeType::from_id(&t.text).ok_or_else(|| format!("key_type: '{}' is not a keyframe type", t.text))?,
                    None => KeyframeType::Keyframe,
                };
                let sel = cmd.b("select")?.unwrap_or(false);
                let layer = &mut scene.grease_pencils[g].layers[l];
                let fi = layer.ensure_key(frame, false);
                layer.frames[fi].key_type = key;
                layer.frames[fi].select = sel;
                Ok(None)
            }
            "gp_stroke" => {
                cmd.max_args(3)?;
                let (g, l) = self.gp_layer_of(scene, cmd)?;
                let frame = num(&cmd.arg(2, "a frame")?.text)? as i32;
                let pts = cmd.list("points")?.ok_or("gp_stroke needs points=")?;
                if pts.len() % 3 != 0 {
                    return Err("points= needs x,y,z triples".into());
                }
                let n = pts.len() / 3;
                let radius = per_point(cmd.list("radius")?, n, 1, "radius")?;
                let opacity = per_point(cmd.list("opacity")?, n, 1, "opacity")?;
                let color = per_point(cmd.list("color")?, n, 4, "color")?;
                let rotation = per_point(cmd.list("rotation")?, n, 1, "rotation")?;
                let select = per_point(cmd.list("select_points")?, n, 1, "select_points")?;
                let mut s = GpStroke::new(Vec::new(), 0);
                s.points = (0..n)
                    .map(|i| {
                        let mut p = GpPoint::new(Vec3::new(pts[i * 3], pts[i * 3 + 1], pts[i * 3 + 2]), radius.as_ref().map_or(0.01, |r| r[i]));
                        if let Some(o) = &opacity {
                            p.opacity = o[i];
                        }
                        if let Some(c) = &color {
                            p.vertex_color = [c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]];
                        }
                        if let Some(r) = &rotation {
                            p.rotation = r[i];
                        }
                        if let Some(sl) = &select {
                            p.select = sl[i] > 0.5;
                        }
                        p
                    })
                    .collect();
                if let Some(w) = cmd.list("weights")? {
                    if w.len() % 3 != 0 {
                        return Err("weights= needs point,group,weight triples".into());
                    }
                    for c in w.chunks(3) {
                        let p = s.points.get_mut(c[0] as usize).ok_or("weights=: point index out of range")?;
                        p.set_weight(c[1] as u32, c[2]);
                    }
                }
                s.cyclic = cmd.b("cyclic")?.unwrap_or(false);
                s.material = cmd.u("material")?.unwrap_or(0) as usize;
                if let Some(c) = cmd.list("fill_color")? {
                    if c.len() != 4 {
                        return Err("fill_color= needs 4 numbers".into());
                    }
                    s.fill_color = [c[0], c[1], c[2], c[3]];
                }
                s.fill_opacity = cmd.f("fill_opacity")?.unwrap_or(s.fill_opacity);
                s.softness = cmd.f("softness")?.unwrap_or(s.softness);
                if let Some(t) = cmd.opt("start_cap") {
                    s.start_cap = cap_of(&t.text)?;
                }
                if let Some(t) = cmd.opt("end_cap") {
                    s.end_cap = cap_of(&t.text)?;
                }
                s.uv_offset = cmd.f("uv_offset")?.unwrap_or(s.uv_offset);
                s.uv_scale = cmd.f("uv_scale")?.unwrap_or(s.uv_scale);
                s.uv_rotation = cmd.f("uv_rotation")?.unwrap_or(s.uv_rotation);
                s.init_time = cmd.f("init_time")?.unwrap_or(s.init_time);
                s.select = cmd.b("select")?.unwrap_or(false);
                let layer = &mut scene.grease_pencils[g].layers[l];
                let fi = layer.ensure_key(frame, false);
                layer.frames[fi].drawing.strokes.push(s);
                Ok(None)
            }
            "add_gpencil" => {
                cmd.max_args(1)?;
                let name = given_name(cmd, true).unwrap_or_else(|| "GPencil".into());
                let g = match cmd.opt("data") {
                    Some(t) => self.gp_data(scene, &t)?,
                    None => scene.add_grease_pencil(&name, GreasePencil::blank()),
                };
                let i = scene.add_object(Object::new(&name, ObjectData::GreasePencil(g)));
                self.created(scene, cmd, i).map(Some)
            }
            "gp_modifier_add" | "gp_effect_add" => {
                cmd.max_args(2)?;
                let o = self.object(scene, cmd.arg(0, "an object")?)?;
                let ty = cmd.arg(1, "a type")?.text.clone();
                let name = cmd.opt("name").map(|t| t.text);
                let ob = &mut scene.objects[o];
                if cmd.name == "gp_modifier_add" {
                    let mut m = GpModifier::new(&ty).ok_or_else(|| format!("unknown grease pencil modifier '{ty}'"))?;
                    if let Some(n) = name {
                        m.name = n;
                    }
                    visit_opts(cmd, |v| m.visit(v))?;
                    ob.gp_modifiers.push(m);
                    Ok(Some((ob.gp_modifiers.len() - 1).to_string()))
                } else {
                    let mut e = GpEffect::new(&ty).ok_or_else(|| format!("unknown visual effect '{ty}'"))?;
                    if let Some(n) = name {
                        e.name = n;
                    }
                    visit_opts(cmd, |v| e.visit(v))?;
                    ob.shader_fx.push(e);
                    Ok(Some((ob.shader_fx.len() - 1).to_string()))
                }
            }
            "seq_set" => {
                cmd.max_args(0)?;
                let q = scene.sequence_editor.get_or_insert_with(Sequencer::default);
                if let Some(v) = cmd.b("use_sequencer")? {
                    q.use_sequencer = v;
                }
                if let Some(t) = cmd.opt("active") {
                    q.active = if t.is_word("none") { None } else { q.find(&t.text) };
                }
                Ok(None)
            }
            "seq_channel" => {
                cmd.max_args(1)?;
                let n = uint(&cmd.arg(0, "a channel number")?.text)?.clamp(1, crate::sequencer::MAX_CHANNELS);
                let q = scene.sequence_editor.get_or_insert_with(Sequencer::default);
                let ch = q.channel_mut(n);
                if let Some(t) = cmd.opt("name") {
                    ch.name = t.text;
                }
                visit_opts(cmd, |v| ch.visit(v))?;
                Ok(None)
            }
            "seq_strip" => {
                cmd.max_args(1)?;
                let name = cmd.arg(0, "a strip name")?.text.clone();
                let ty = cmd.opt("type").map(|t| t.text.to_ascii_lowercase()).unwrap_or_else(|| "color".into());
                let kind = match ty.as_str() {
                    "color" => StripKind::Color { color: [1.0; 3] },
                    "image" => StripKind::Image { directory: String::new(), files: Vec::new() },
                    "movie" => StripKind::Movie { path: String::new() },
                    "sound" => StripKind::Sound { path: String::new(), volume: 1.0, pan: 0.0, waveform: Vec::new() },
                    "scene" => StripKind::Scene { scene: String::new(), camera: String::new() },
                    "text" => StripKind::Text(Default::default()),
                    "adjustment" => StripKind::Adjustment,
                    "meta" => StripKind::Meta { strips: Vec::new() },
                    e => StripKind::Effect { effect: Effect::new(e).ok_or_else(|| format!("unknown strip type '{e}'"))?, inputs: Vec::new(), fader: None },
                };
                let parent = cmd.opt("meta").map(|t| t.text);
                let select = cmd.b("select")?.unwrap_or(false);
                let mut s = Strip::new(&name, kind, 1, 1.0, 1.0);
                s.select = select;
                visit_opts(cmd, |v| {
                    s.visit_time(v);
                    s.visit_look(v);
                    s.visit_kind(v);
                })?;
                let q = scene.sequence_editor.get_or_insert_with(Sequencer::default);
                match parent {
                    Some(p) => match find_strip(&mut q.strips, &p).map(|m| &mut m.kind) {
                        Some(StripKind::Meta { strips }) => strips.push(s),
                        _ => return Err(format!("no meta strip named '{p}'")),
                    },
                    None => q.strips.push(s),
                }
                Ok(None)
            }
            "seq_modifier_add" => {
                cmd.max_args(2)?;
                let name = cmd.arg(0, "a strip")?.text.clone();
                let ty = cmd.arg(1, "a type")?.text.clone();
                let mut m = StripModifier::new(&ty).ok_or_else(|| format!("unknown strip modifier '{ty}'"))?;
                if let Some(t) = cmd.opt("name") {
                    m.name = t.text;
                }
                visit_opts(cmd, |v| m.visit(v))?;
                let q = scene.sequence_editor.get_or_insert_with(Sequencer::default);
                let s = find_strip(&mut q.strips, &name).ok_or_else(|| format!("no strip named '{name}'"))?;
                s.modifiers.push(m);
                Ok(None)
            }
            other => Err(format!("unknown command '{other}'")),
        }
    }
}

// ---------------------------------------------------------------------------
// Writing

/// Grease Pencil data-blocks (before the objects).
pub(super) fn write_data(o: &mut String, scene: &Scene) {
    for (g, name) in scene.grease_pencils.iter().zip(&scene.grease_pencil_names) {
        let n = quote(name);
        let mut g = g.clone();
        let _ = writeln!(o, "gp_new {n}");
        for vg in &g.vertex_groups {
            let _ = writeln!(o, "gp_vgroup {n} {}", quote(vg));
        }
        for m in g.materials.iter_mut() {
            let _ = writeln!(o, "gp_material {n} {}{}", quote(&m.name), opts_of(|v| m.visit(v)));
        }
        for gr in g.groups.iter_mut() {
            let parent = gr.parent.map_or("none".to_string(), |p| p.to_string());
            let _ = writeln!(o, "gp_group {n} {} parent={parent}{}", quote(&gr.name), opts_of(|v| gr.visit(v)));
        }
        for l in g.layers.iter_mut() {
            let ln = quote(&l.name);
            let group = l.group.map_or("none".to_string(), |p| p.to_string());
            let _ = writeln!(o, "gp_layer {n} {ln} group={group}{}", opts_of(|v| l.visit(v)));
            for f in &l.frames {
                let sel = if f.select { " select=1" } else { "" };
                let _ = writeln!(o, "gp_frame {n} {ln} {} key_type={}{sel}", f.frame, f.key_type.id().to_ascii_lowercase());
                for s in &f.drawing.strokes {
                    let _ = writeln!(o, "gp_stroke {n} {ln} {}{}", f.frame, stroke_options(s));
                }
            }
        }
        let (al, am, av) = (g.active_layer, g.active_material, g.active_vertex_group);
        let _ = writeln!(o, "gp_set {n} active_layer={al} active_material={am} active_vertex_group={av}{}", opts_of(|v| g.visit(v)));
    }
}

fn stroke_options(s: &GpStroke) -> String {
    let n = s.points.len();
    let mut o = String::new();
    let pos: Vec<f32> = s.points.iter().flat_map(|p| p.position.to_array()).collect();
    let list = |v: &[f32]| if v.len() > 12 { b64_f32(v) } else { fmt_list(v) };
    // One value stands for all when every point agrees.
    let col = |f: &dyn Fn(&GpPoint) -> Vec<f32>| -> String {
        let first = s.points.first().map(f).unwrap_or_default();
        if s.points.iter().all(|p| f(p) == first) {
            fmt_list(&first)
        } else {
            list(&s.points.iter().flat_map(f).collect::<Vec<_>>())
        }
    };
    let _ = write!(o, " points={}", list(&pos));
    if n > 0 {
        let _ = write!(o, " radius={}", col(&|p| vec![p.radius]));
        let _ = write!(o, " opacity={}", col(&|p| vec![p.opacity]));
        let c = col(&|p| p.vertex_color.to_vec());
        if c != "0,0,0,0" {
            let _ = write!(o, " color={c}");
        }
        let r = col(&|p| vec![p.rotation]);
        if r != "0" {
            let _ = write!(o, " rotation={r}");
        }
        if s.points.iter().any(|p| p.select) {
            let _ = write!(o, " select_points={}", col(&|p| vec![p.select as u8 as f32]));
        }
    }
    let w: Vec<f32> = s.points.iter().enumerate().flat_map(|(i, p)| p.weights.iter().flat_map(move |&(g, w)| [i as f32, g as f32, w])).collect();
    if !w.is_empty() {
        let _ = write!(o, " weights={}", fmt_list(&w));
    }
    let d = GpStroke::new(Vec::new(), 0);
    let _ = write!(o, " material={}", s.material);
    if s.cyclic {
        o.push_str(" cyclic=1");
    }
    if s.fill_color != d.fill_color {
        let _ = write!(o, " fill_color={}", fmt_list(&s.fill_color));
    }
    for (k, a, b) in [("fill_opacity", s.fill_opacity, d.fill_opacity), ("softness", s.softness, d.softness), ("uv_offset", s.uv_offset, d.uv_offset), ("uv_scale", s.uv_scale, d.uv_scale), ("uv_rotation", s.uv_rotation, d.uv_rotation), ("init_time", s.init_time, d.init_time)] {
        if a != b {
            let _ = write!(o, " {k}={}", fmt_f(a));
        }
    }
    if s.start_cap != CapType::Round {
        let _ = write!(o, " start_cap={}", cap_name(s.start_cap));
    }
    if s.end_cap != CapType::Round {
        let _ = write!(o, " end_cap={}", cap_name(s.end_cap));
    }
    if s.select {
        o.push_str(" select=1");
    }
    o
}

/// The `add_gpencil` line of a Grease Pencil object.
pub(super) fn object_line(scene: &Scene, ob: &Object, place: &str) -> Option<String> {
    match ob.data {
        ObjectData::GreasePencil(g) => Some(format!("add_gpencil {} data={}{place}", quote(&ob.name), quote(&scene.grease_pencil_names[g]))),
        _ => None,
    }
}

/// Grease Pencil modifiers and visual effects of an object.
pub(super) fn write_object(o: &mut String, ob: &Object) {
    let name = quote(&ob.name);
    for m in &ob.gp_modifiers {
        let mut m = m.clone();
        let _ = writeln!(o, "gp_modifier_add {name} {} name={}{}", m.type_id().to_ascii_lowercase(), quote(&m.name), opts_of(|v| m.visit(v)));
    }
    for e in &ob.shader_fx {
        let mut e = e.clone();
        let _ = writeln!(o, "gp_effect_add {name} {} name={}{}", e.type_id().to_ascii_lowercase(), quote(&e.name), opts_of(|v| e.visit(v)));
    }
}

fn strip_type(s: &Strip) -> String {
    s.kind.type_id().to_ascii_lowercase()
}

fn write_strips(o: &mut String, strips: &[Strip], parent: Option<&str>) {
    for s in strips {
        let mut s = s.clone();
        let meta = parent.map(|p| format!(" meta={}", quote(p))).unwrap_or_default();
        let sel = if s.select { " select=1" } else { "" };
        let ty = strip_type(&s);
        let opts = opts_of(|v| {
            s.visit_time(v);
            s.visit_look(v);
            s.visit_kind(v);
        });
        let _ = writeln!(o, "seq_strip {} type={ty}{meta}{sel}{opts}", quote(&s.name));
        for m in &s.modifiers {
            let mut m = m.clone();
            let _ = writeln!(o, "seq_modifier_add {} {} name={}{}", quote(&s.name), m.type_id().to_ascii_lowercase(), quote(&m.name), opts_of(|v| m.visit(v)));
        }
        if let StripKind::Meta { strips: inner } = &s.kind {
            write_strips(o, inner, Some(&s.name));
        }
    }
}

/// The sequence editor.
pub(super) fn write_sequencer(o: &mut String, scene: &Scene) {
    let Some(q) = &scene.sequence_editor else { return };
    let active = q.active.and_then(|i| q.strips.get(i)).map_or("none".to_string(), |s| quote(&s.name));
    let _ = writeln!(o, "seq_set use_sequencer={}", q.use_sequencer as u8);
    for (i, ch) in q.channels.iter().enumerate() {
        if ch.lock || ch.mute || ch.name != format!("Channel {}", i + 1) {
            let _ = writeln!(o, "seq_channel {} name={} lock={} mute={}", i + 1, quote(&ch.name), ch.lock as u8, ch.mute as u8);
        }
    }
    write_strips(o, &q.strips, None);
    let _ = writeln!(o, "seq_set active={active}");
}
