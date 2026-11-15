//! Polyamour's native `.poly` file: the scene as polyops commands, followed by
//! the document state polyops has no words for (animation, render and scene
//! settings, text blocks, the current frame). That state rides in `#@` comment
//! lines, so the file stays a valid polyops script that `sex_3d::script::run`
//! (and the Python package) still read as a plain scene.

use crate::anim::{Interp, Keyframe, Prop};
use crate::doc::Doc;
use crate::settings::{Engine, RotationMode, Settings, UnitSystem};
use sex_3d::render::ViewTransform;
use sex_3d::scene::Scene;

/// The whole document as `.poly` text.
pub fn save(doc: &Doc) -> String {
    // Modifiers switched off in the viewport are parked outside the stack;
    // write them back in place so the file keeps them.
    let mut scene = doc.scene.clone();
    for p in &doc.settings.parked {
        if let Some(o) = scene.objects.iter_mut().find(|o| o.name == p.object) {
            let at = p.slot.min(o.modifiers.len());
            o.modifiers.insert(at, p.modifier.clone());
        }
    }
    let mut out = sex_3d::script::scene_to_polyops(&scene);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("#@polyamour 1\n");
    out.push_str(&format!("#@frame {}\n", doc.frame));
    let a = &doc.anim;
    out.push_str(&format!("#@anim {} {} {}\n", a.auto_key as u8, a.reverse as u8, interp_name(a.interp)));
    for o in &a.objects {
        for c in &o.curves {
            for k in &c.keys {
                out.push_str(&format!(
                    "#@key {}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    esc(&o.object),
                    c.prop.path(),
                    c.index,
                    k.frame,
                    k.value,
                    interp_name(k.interp),
                    k.left[0],
                    k.left[1],
                    k.right[0],
                    k.right[1]
                ));
            }
        }
    }
    for (k, v) in settings_pairs(&doc.settings) {
        out.push_str(&format!("#@set {k}\t{}\n", esc(&v)));
    }
    for (name, body) in &doc.texts {
        out.push_str(&format!("#@text {}\t{}\n", esc(name), esc(body)));
    }
    out
}

/// Read a `.poly` file back into a document.
pub fn load(text: &str) -> Result<Doc, String> {
    let mut scene = Scene::new();
    sex_3d::script::run(&mut scene, text).map_err(|e| e.to_string())?;
    let mut doc = Doc::new(scene);
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("#@") else { continue };
        let (tag, body) = rest.split_once(' ').unwrap_or((rest, ""));
        match tag {
            "frame" => {
                if let Ok(f) = body.trim().parse() {
                    doc.frame = f;
                }
            }
            "anim" => {
                let mut w = body.split_whitespace();
                doc.anim.auto_key = w.next() == Some("1");
                doc.anim.reverse = w.next() == Some("1");
                if let Some(i) = w.next().and_then(Interp::from_arg) {
                    doc.anim.interp = i;
                }
            }
            "key" => {
                let f: Vec<&str> = body.split('\t').collect();
                if f.len() < 10 {
                    continue;
                }
                let Some(prop) = Prop::ALL.into_iter().find(|p| p.path() == f[1]) else { continue };
                let (Ok(index), Some(interp)) = (f[2].parse::<usize>(), Interp::from_arg(f[5])) else { continue };
                let n: Vec<f32> = [f[3], f[4], f[6], f[7], f[8], f[9]].iter().map(|s| s.parse().unwrap_or(0.0)).collect();
                let obj = doc.anim.entry(&unesc(f[0]));
                let curve = match obj.curves.iter().position(|c| c.prop == prop && c.index == index) {
                    Some(i) => &mut obj.curves[i],
                    None => {
                        obj.curves.push(crate::anim::FCurve::new(prop, index.min(2)));
                        obj.curves.sort_by_key(|c| (c.prop, c.index));
                        let i = obj.curves.iter().position(|c| c.prop == prop && c.index == index.min(2)).unwrap();
                        &mut obj.curves[i]
                    }
                };
                curve.keys.push(Keyframe { frame: n[0], value: n[1], interp, left: [n[2], n[3]], right: [n[4], n[5]], select: false });
                curve.sort();
            }
            "set" => {
                if let Some((k, v)) = body.split_once('\t') {
                    set_setting(&mut doc.settings, k, &unesc(v));
                }
            }
            "text" => {
                if let Some((name, b)) = body.split_once('\t') {
                    doc.texts.push((unesc(name), unesc(b)));
                }
            }
            _ => {}
        }
    }
    doc.anim.dirty = true;
    Ok(doc)
}

fn interp_name(i: Interp) -> &'static str {
    match i {
        Interp::Constant => "CONSTANT",
        Interp::Linear => "LINEAR",
        Interp::Bezier => "BEZIER",
    }
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

fn unesc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some('t') => o.push('\t'),
            Some(c) => o.push(c),
            None => o.push('\\'),
        }
    }
    o
}

fn join<T: ToString>(v: impl IntoIterator<Item = T>) -> String {
    v.into_iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

fn rot_name(r: RotationMode) -> &'static str {
    match r {
        RotationMode::Quaternion => "QUATERNION",
        RotationMode::EulerXyz => "XYZ",
        RotationMode::AxisAngle => "AXIS_ANGLE",
    }
}

fn settings_pairs(s: &Settings) -> Vec<(&'static str, String)> {
    let b = |v: bool| (v as u8).to_string();
    vec![
        ("engine", match s.engine { Engine::PathTracer => "PATH", Engine::Workbench => "WORKBENCH" }.into()),
        ("viewport_samples", s.viewport_samples.to_string()),
        ("render_samples", s.render_samples.to_string()),
        ("noise_threshold", b(s.noise_threshold)),
        ("max_bounces", s.max_bounces.to_string()),
        ("denoise", b(s.denoise)),
        ("viewport_denoise", b(s.viewport_denoise)),
        ("clamp_indirect", s.clamp_indirect.to_string()),
        ("seed", s.seed.to_string()),
        ("film_transparent", b(s.film_transparent)),
        ("filter_width", s.filter_width.to_string()),
        ("resolution_x", s.resolution_x.to_string()),
        ("resolution_y", s.resolution_y.to_string()),
        ("resolution_pct", s.resolution_pct.to_string()),
        ("frame_step", s.frame_step.to_string()),
        ("output_path", s.output_path.clone()),
        ("file_format", s.file_format.clone()),
        ("color_mode", s.color_mode.clone()),
        ("unit_system", match s.unit_system { UnitSystem::None => "NONE", UnitSystem::Metric => "METRIC", UnitSystem::Imperial => "IMPERIAL" }.into()),
        ("unit_scale", s.unit_scale.to_string()),
        ("degrees", b(s.degrees)),
        ("gravity_on", b(s.gravity_on)),
        ("gravity", join(s.gravity)),
        ("view_transform", match s.view_transform { ViewTransform::Standard => "STANDARD", ViewTransform::AgX => "AGX" }.into()),
        ("look", s.look.clone()),
        ("exposure", s.exposure.to_string()),
        ("gamma", s.gamma.to_string()),
        ("use_for_rendering", b(s.use_for_rendering)),
        ("pass_z", b(s.pass_z)),
        ("pass_normal", b(s.pass_normal)),
        ("pass_denoising", b(s.pass_denoising)),
        ("hide_render", s.hide_render.iter().map(|n| esc(n).replace(',', "\\,")).collect::<Vec<_>>().join(",")),
        ("excluded", join(&s.excluded)),
        ("collection_hidden", join(&s.collection_hidden)),
        ("collection_hide_render", join(&s.collection_hide_render)),
        ("rotation_mode", s.rotation_mode.iter().map(|(n, r)| format!("{}={}", esc(n).replace([',', '='], "_"), rot_name(*r))).collect::<Vec<_>>().join(",")),
        ("sensor", s.sensor.iter().map(|(n, v)| format!("{}={v}", esc(n).replace([',', '='], "_"))).collect::<Vec<_>>().join(",")),
        ("active_collection", s.active_collection.to_string()),
    ]
}

fn set_setting(s: &mut Settings, key: &str, v: &str) {
    fn p<T: std::str::FromStr>(dst: &mut T, v: &str) {
        if let Ok(x) = v.parse() {
            *dst = x;
        }
    }
    let flag = |dst: &mut bool| *dst = v == "1";
    let list = |v: &str| -> Vec<String> {
        // Split on commas not preceded by a backslash.
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut it = v.chars();
        while let Some(c) = it.next() {
            match c {
                '\\' => {
                    if let Some(n) = it.next() {
                        if n == ',' {
                            cur.push(',');
                        } else {
                            cur.push('\\');
                            cur.push(n);
                        }
                    }
                }
                ',' => out.push(std::mem::take(&mut cur)),
                c => cur.push(c),
            }
        }
        if !cur.is_empty() || !out.is_empty() {
            out.push(cur);
        }
        out
    };
    match key {
        "engine" => s.engine = if v == "WORKBENCH" { Engine::Workbench } else { Engine::PathTracer },
        "viewport_samples" => p(&mut s.viewport_samples, v),
        "render_samples" => p(&mut s.render_samples, v),
        "noise_threshold" => flag(&mut s.noise_threshold),
        "max_bounces" => p(&mut s.max_bounces, v),
        "denoise" => flag(&mut s.denoise),
        "viewport_denoise" => flag(&mut s.viewport_denoise),
        "clamp_indirect" => p(&mut s.clamp_indirect, v),
        "seed" => p(&mut s.seed, v),
        "film_transparent" => flag(&mut s.film_transparent),
        "filter_width" => p(&mut s.filter_width, v),
        "resolution_x" => p(&mut s.resolution_x, v),
        "resolution_y" => p(&mut s.resolution_y, v),
        "resolution_pct" => p(&mut s.resolution_pct, v),
        "frame_step" => p(&mut s.frame_step, v),
        "output_path" => s.output_path = v.into(),
        "file_format" => s.file_format = v.into(),
        "color_mode" => s.color_mode = v.into(),
        "unit_system" => {
            s.unit_system = match v {
                "NONE" => UnitSystem::None,
                "IMPERIAL" => UnitSystem::Imperial,
                _ => UnitSystem::Metric,
            }
        }
        "unit_scale" => p(&mut s.unit_scale, v),
        "degrees" => flag(&mut s.degrees),
        "gravity_on" => flag(&mut s.gravity_on),
        "gravity" => {
            let g: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            if g.len() == 3 {
                s.gravity = [g[0], g[1], g[2]];
            }
        }
        "view_transform" => s.view_transform = if v == "STANDARD" { ViewTransform::Standard } else { ViewTransform::AgX },
        "look" => s.look = v.into(),
        "exposure" => p(&mut s.exposure, v),
        "gamma" => p(&mut s.gamma, v),
        "use_for_rendering" => flag(&mut s.use_for_rendering),
        "pass_z" => flag(&mut s.pass_z),
        "pass_normal" => flag(&mut s.pass_normal),
        "pass_denoising" => flag(&mut s.pass_denoising),
        "hide_render" => s.hide_render = list(v).into_iter().map(|n| unesc(&n)).collect(),
        "excluded" => s.excluded = v.split(',').filter_map(|x| x.parse().ok()).collect(),
        "collection_hidden" => s.collection_hidden = v.split(',').filter_map(|x| x.parse().ok()).collect(),
        "collection_hide_render" => s.collection_hide_render = v.split(',').filter_map(|x| x.parse().ok()).collect(),
        "rotation_mode" => {
            for pair in v.split(',') {
                if let Some((n, r)) = pair.split_once('=') {
                    let mode = match r {
                        "QUATERNION" => RotationMode::Quaternion,
                        "AXIS_ANGLE" => RotationMode::AxisAngle,
                        _ => RotationMode::EulerXyz,
                    };
                    s.rotation_mode.insert(unesc(n), mode);
                }
            }
        }
        "sensor" => {
            for pair in v.split(',') {
                if let Some((n, x)) = pair.split_once('=') {
                    if let Ok(x) = x.parse() {
                        s.sensor.insert(unesc(n), x);
                    }
                }
            }
        }
        "active_collection" => p(&mut s.active_collection, v),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_round_trips_document_state() {
        let mut doc = Doc::default();
        doc.frame = 12;
        crate::anim::insert_keys(&mut doc, 12, None);
        doc.settings.render_samples = 99;
        doc.settings.engine = Engine::Workbench;
        doc.settings.hide_render.insert("Cube, the first".into());
        doc.settings.sensor.insert("Camera".into(), 50.0);
        doc.texts.push(("Text".into(), "line one\n\tline two \\ end".into()));
        let keys_before = doc.anim.objects.iter().map(|o| o.curves.iter().map(|c| c.keys.len()).sum::<usize>()).sum::<usize>();
        assert!(keys_before > 0, "default scene has a selected object to key");

        let text = save(&doc);
        let back = load(&text).unwrap();
        assert_eq!(back.frame, 12);
        assert_eq!(back.settings, doc.settings);
        assert_eq!(back.texts, doc.texts);
        assert_eq!(back.anim.objects.len(), doc.anim.objects.len());
        for (a, b) in back.anim.objects.iter().zip(&doc.anim.objects) {
            assert_eq!(a.object, b.object);
            assert_eq!(a.curves.len(), b.curves.len());
            for (ca, cb) in a.curves.iter().zip(&b.curves) {
                assert_eq!((ca.prop, ca.index), (cb.prop, cb.index));
                let strip = |k: &Keyframe| (k.frame, k.value, k.interp, k.left, k.right);
                assert_eq!(ca.keys.iter().map(strip).collect::<Vec<_>>(), cb.keys.iter().map(strip).collect::<Vec<_>>());
            }
        }
        // Still a plain polyops script for other readers.
        let mut s = Scene::new();
        sex_3d::script::run(&mut s, &text).unwrap();
        assert_eq!(s.objects.len(), doc.scene.objects.len());
    }

    #[test]
    fn rename_keeps_animation_and_app_reopens_poly() {
        let mut app = crate::app::App::new(320, 200);
        app.doc.frame = 5;
        crate::anim::insert_keys(&mut app.doc, 5, None);
        let name = app.doc.anim.objects[0].object.clone();
        let o = app.doc.scene.objects.iter().position(|x| x.name == name).unwrap();
        crate::editors::properties::rename_object(&mut app.doc, o, "Renamed");
        assert_eq!(app.doc.anim.objects[0].object, "Renamed");
        let text = save(&app.doc);
        app.open_bytes("untitled.poly", text.as_bytes(), &|_| None).unwrap();
        assert!(app.doc.anim.get("Renamed").is_some());
        // Sniffed by content too.
        app.open_bytes("noext", text.as_bytes(), &|_| None).unwrap();
        assert!(app.doc.anim.get("Renamed").is_some());
    }
}
