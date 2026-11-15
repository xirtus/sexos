//! Polyamour's native `.poly` file: the scene as polyops commands, followed by
//! the document state polyops has no words for (animation, render and scene
//! settings, text blocks, the current frame). That state rides in `#@` comment
//! lines, so the file stays a valid polyops script that `sex_3d::script::run`
//! (and the Python package) still read as a plain scene.

use crate::anim::{Interp, Keyframe, Prop};
use crate::doc::Doc;
use crate::settings::{Engine, RotationMode, Settings, UnitSystem};
use sex_3d::render::{MistFalloff, Pass, Passes, ShutterPosition, ViewTransform};
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
    trees::save(&doc.scene, &mut out);
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
    doc.settings.migrate_sensors(&mut doc.scene);
    trees::load(&mut doc.scene, text);
    doc.anim.dirty = true;
    Ok(doc)
}

mod trees;

fn interp_name(i: Interp) -> &'static str {
    match i {
        Interp::Constant => "CONSTANT",
        Interp::Linear => "LINEAR",
        Interp::Bezier => "BEZIER",
    }
}

pub(crate) fn esc(s: &str) -> String {
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

pub(crate) fn unesc(s: &str) -> String {
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

/// Pass names in `.poly` files (Blender's `use_pass_*` property suffixes).
fn pass_key(p: Pass) -> &'static str {
    match p {
        Pass::Combined => "combined",
        Pass::Depth => "z",
        Pass::Mist => "mist",
        Pass::Normal => "normal",
        Pass::Position => "position",
        Pass::Uv => "uv",
        Pass::DenoisingAlbedo => "denoising_albedo",
        Pass::DenoisingNormal => "denoising_normal",
        Pass::ObjectIndex => "object_index",
        Pass::MaterialIndex => "material_index",
        Pass::DiffuseDirect => "diffuse_direct",
        Pass::DiffuseIndirect => "diffuse_indirect",
        Pass::DiffuseColor => "diffuse_color",
        Pass::GlossyDirect => "glossy_direct",
        Pass::GlossyIndirect => "glossy_indirect",
        Pass::GlossyColor => "glossy_color",
        Pass::TransmissionDirect => "transmission_direct",
        Pass::TransmissionIndirect => "transmission_indirect",
        Pass::TransmissionColor => "transmission_color",
        Pass::VolumeDirect => "volume_direct",
        Pass::VolumeIndirect => "volume_indirect",
        Pass::Emission => "emit",
        Pass::Environment => "environment",
        Pass::AmbientOcclusion => "ambient_occlusion",
    }
}

fn view_key(v: ViewTransform) -> &'static str {
    match v {
        ViewTransform::Standard => "STANDARD",
        ViewTransform::Filmic => "FILMIC",
        ViewTransform::AgX => "AGX",
        ViewTransform::KhronosPbrNeutral => "KHRONOS_PBR_NEUTRAL",
        ViewTransform::FalseColor => "FALSE_COLOR",
        ViewTransform::Raw => "RAW",
    }
}

fn settings_pairs(s: &Settings) -> Vec<(&'static str, String)> {
    let b = |v: bool| (v as u8).to_string();
    vec![
        ("engine", match s.engine { Engine::PathTracer => "PATH", Engine::Workbench => "WORKBENCH" }.into()),
        ("viewport_samples", s.viewport_samples.to_string()),
        ("render_samples", s.render_samples.to_string()),
        ("noise_threshold", b(s.noise_threshold)),
        ("adaptive_threshold", s.adaptive_threshold.to_string()),
        ("adaptive_min_samples", s.adaptive_min_samples.to_string()),
        ("viewport_noise_threshold", b(s.viewport_noise_threshold)),
        ("viewport_adaptive_threshold", s.viewport_adaptive_threshold.to_string()),
        ("time_limit", s.time_limit.to_string()),
        ("max_bounces", s.max_bounces.to_string()),
        ("diffuse_bounces", s.diffuse_bounces.to_string()),
        ("glossy_bounces", s.glossy_bounces.to_string()),
        ("transmission_bounces", s.transmission_bounces.to_string()),
        ("volume_bounces", s.volume_bounces.to_string()),
        ("transparent_bounces", s.transparent_bounces.to_string()),
        ("denoise", b(s.denoise)),
        ("viewport_denoise", b(s.viewport_denoise)),
        ("clamp_direct", s.clamp_direct.to_string()),
        ("clamp_indirect", s.clamp_indirect.to_string()),
        ("caustics_reflective", b(s.caustics_reflective)),
        ("caustics_refractive", b(s.caustics_refractive)),
        ("filter_glossy", s.filter_glossy.to_string()),
        ("seed", s.seed.to_string()),
        ("animated_seed", b(s.animated_seed)),
        ("film_transparent", b(s.film_transparent)),
        ("filter_width", s.filter_width.to_string()),
        ("threads", s.threads.to_string()),
        ("motion_blur", b(s.motion_blur)),
        ("shutter", s.shutter.to_string()),
        ("shutter_position", match s.shutter_position { ShutterPosition::Start => "START", ShutterPosition::Center => "CENTER", ShutterPosition::End => "END" }.into()),
        ("use_border", b(s.use_border)),
        ("border", join(s.border)),
        ("crop_to_border", b(s.crop_to_border)),
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
        ("view_transform", view_key(s.view_transform).into()),
        ("look", s.look.clone()),
        ("exposure", s.exposure.to_string()),
        ("gamma", s.gamma.to_string()),
        ("use_for_rendering", b(s.use_for_rendering)),
        ("passes", s.passes.iter().map(pass_key).collect::<Vec<_>>().join(",")),
        ("mist_start", s.mist_start.to_string()),
        ("mist_depth", s.mist_depth.to_string()),
        ("mist_falloff", match s.mist_falloff { MistFalloff::Quadratic => "QUADRATIC", MistFalloff::Linear => "LINEAR", MistFalloff::InverseQuadratic => "INVERSE_QUADRATIC" }.into()),
        ("ao_distance", s.ao_distance.to_string()),
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
        "adaptive_threshold" => p(&mut s.adaptive_threshold, v),
        "adaptive_min_samples" => p(&mut s.adaptive_min_samples, v),
        "viewport_noise_threshold" => flag(&mut s.viewport_noise_threshold),
        "viewport_adaptive_threshold" => p(&mut s.viewport_adaptive_threshold, v),
        "time_limit" => p(&mut s.time_limit, v),
        "max_bounces" => p(&mut s.max_bounces, v),
        "diffuse_bounces" => p(&mut s.diffuse_bounces, v),
        "glossy_bounces" => p(&mut s.glossy_bounces, v),
        "transmission_bounces" => p(&mut s.transmission_bounces, v),
        "volume_bounces" => p(&mut s.volume_bounces, v),
        "transparent_bounces" => p(&mut s.transparent_bounces, v),
        "denoise" => flag(&mut s.denoise),
        "viewport_denoise" => flag(&mut s.viewport_denoise),
        "clamp_direct" => p(&mut s.clamp_direct, v),
        "clamp_indirect" => p(&mut s.clamp_indirect, v),
        "caustics_reflective" => flag(&mut s.caustics_reflective),
        "caustics_refractive" => flag(&mut s.caustics_refractive),
        "filter_glossy" => p(&mut s.filter_glossy, v),
        "seed" => p(&mut s.seed, v),
        "animated_seed" => flag(&mut s.animated_seed),
        "film_transparent" => flag(&mut s.film_transparent),
        "filter_width" => p(&mut s.filter_width, v),
        "threads" => p(&mut s.threads, v),
        "motion_blur" => flag(&mut s.motion_blur),
        "shutter" => p(&mut s.shutter, v),
        "shutter_position" => {
            s.shutter_position = match v {
                "START" => ShutterPosition::Start,
                "END" => ShutterPosition::End,
                _ => ShutterPosition::Center,
            }
        }
        "use_border" => flag(&mut s.use_border),
        "border" => {
            let b: Vec<f32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            if b.len() == 4 {
                s.border = [b[0], b[1], b[2], b[3]];
            }
        }
        "crop_to_border" => flag(&mut s.crop_to_border),
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
        "view_transform" => s.view_transform = ViewTransform::ALL.into_iter().find(|t| view_key(*t) == v).unwrap_or(ViewTransform::AgX),
        "look" => s.look = v.into(),
        "exposure" => p(&mut s.exposure, v),
        "gamma" => p(&mut s.gamma, v),
        "use_for_rendering" => flag(&mut s.use_for_rendering),
        "passes" => s.passes = v.split(',').filter_map(|k| Pass::ALL.into_iter().find(|p| pass_key(*p) == k)).collect::<Passes>(),
        // Older files.
        "pass_z" => s.passes.set(Pass::Depth, v == "1"),
        "pass_normal" => s.passes.set(Pass::Normal, v == "1"),
        "pass_denoising" => {
            s.passes.set(Pass::DenoisingAlbedo, v == "1");
            s.passes.set(Pass::DenoisingNormal, v == "1");
        }
        "mist_start" => p(&mut s.mist_start, v),
        "mist_depth" => p(&mut s.mist_depth, v),
        "mist_falloff" => {
            s.mist_falloff = match v {
                "LINEAR" => MistFalloff::Linear,
                "INVERSE_QUADRATIC" => MistFalloff::InverseQuadratic,
                _ => MistFalloff::Quadratic,
            }
        }
        "ao_distance" => p(&mut s.ao_distance, v),
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
    fn old_files_migrate_sensor_sizes_and_passes() {
        let doc = Doc::default();
        let cam = doc.scene.objects.iter().find(|o| matches!(o.data, sex_3d::scene::ObjectData::Camera(_))).unwrap().name.clone();
        let mut text = save(&doc);
        text.push_str(&format!("#@set sensor\t{cam}=50\n#@set pass_z\t0\n#@set pass_normal\t1\n#@set pass_denoising\t1\n"));
        let back = load(&text).unwrap();
        assert!(back.settings.sensor.is_empty());
        let o = back.scene.objects.iter().find(|o| o.name == cam).unwrap();
        let sex_3d::scene::ObjectData::Camera(c) = &o.data else { panic!() };
        assert_eq!(c.sensor_width, 50.0);
        let p = back.settings.passes;
        assert!(!p.contains(Pass::Depth) && p.contains(Pass::Normal) && p.contains(Pass::DenoisingAlbedo) && p.contains(Pass::DenoisingNormal));
    }

    #[test]
    fn camera_light_and_object_render_options_survive_poly() {
        use sex_3d::scene::{AreaShape, Light, LightKind, ObjectData, PanoramaType};
        let mut doc = Doc::default();
        let ci = doc.scene.objects.iter().position(|o| matches!(o.data, ObjectData::Camera(_))).unwrap();
        if let ObjectData::Camera(c) = &mut doc.scene.objects[ci].data {
            c.panorama = Some(PanoramaType::FisheyeEquisolid);
            c.shift_x = 0.25;
            c.dof.enabled = true;
            c.dof.fstop = 1.4;
            c.dof.blades = 6;
            c.dof.focus_object = Some(0);
        }
        let li = doc.scene.objects.iter().position(|o| matches!(o.data, ObjectData::Light(_))).unwrap();
        doc.scene.objects[li].data = ObjectData::Light(Light { kind: LightKind::Area { size: 2.0 }, shape: AreaShape::Ellipse, size_y: 0.5, spread: 1.0, max_bounces: 3, cast_shadow: false, use_mis: false, ..Light::default() });
        doc.scene.objects[0].holdout = true;
        doc.scene.objects[0].pass_index = 4;
        doc.scene.objects[0].visibility.glossy = false;
        let back = load(&save(&doc)).unwrap();
        assert_eq!(back.scene.objects[ci].data, doc.scene.objects[ci].data);
        assert_eq!(back.scene.objects[li].data, doc.scene.objects[li].data);
        assert!(back.scene.objects[0].holdout);
        assert_eq!(back.scene.objects[0].pass_index, 4);
        assert_eq!(back.scene.objects[0].visibility, doc.scene.objects[0].visibility);
    }

    #[test]
    fn poly_round_trips_document_state() {
        let mut doc = Doc::default();
        doc.frame = 12;
        crate::anim::insert_keys(&mut doc, 12, None);
        doc.settings.render_samples = 99;
        doc.settings.engine = Engine::Workbench;
        doc.settings.hide_render.insert("Cube, the first".into());
        doc.settings.diffuse_bounces = 7;
        doc.settings.caustics_refractive = false;
        doc.settings.view_transform = ViewTransform::KhronosPbrNeutral;
        doc.settings.look = "High Contrast".into();
        doc.settings.passes = [Pass::Mist, Pass::ObjectIndex, Pass::GlossyIndirect].into_iter().collect();
        doc.settings.motion_blur = true;
        doc.settings.shutter_position = ShutterPosition::End;
        doc.settings.use_border = true;
        doc.settings.border = [0.1, 0.2, 0.6, 0.9];
        doc.settings.mist_falloff = MistFalloff::Linear;
        doc.settings.time_limit = 12.5;
        doc.settings.threads = 3;
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
