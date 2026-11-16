// SPDX-License-Identifier: GPL-3.0-only
//! Video Sequencer operators and rendering: adding strips and effects,
//! the edit operations (move, split, duplicate, delete, meta, mute, lock,
//! snap), strip modifiers, and rendering the sequencer's output for the
//! preview and for F12 / Ctrl+F12 (with Grease Pencil composited into the
//! 3D render first). The strip model and its maths are
//! `sex_3d::sequencer`; the editor is `editors/sequencer.rs`.
//!
//! Movie strips: no video decoder is linked into Polyamour, so they show a
//! placeholder. Sound strips read WAV files for their length and waveform;
//! there is no audio playback.
use crate::doc::Doc;
use crate::ui::Op;
use sex_3d::render::{Pass, RenderResult, RenderSettings};
use sex_3d::scene::{Image, ObjectData, Scene};
use sex_3d::sequencer::{render_frame, wav_info, Effect, RenderCtx, Sequencer, Strip, StripKind, StripModifier, TextStrip};
use std::collections::HashMap;

/// The scene's sequence editor, created on first use.
pub fn seq(d: &mut Doc) -> &mut Sequencer {
    d.scene.sequence_editor.get_or_insert_with(Sequencer::default)
}

/// Sequencer output is used for renders: the toggle is on and a strip would draw.
pub fn active(scene: &Scene) -> bool {
    scene.sequence_editor.as_ref().is_some_and(|q| q.use_sequencer && q.has_visual_strips())
}

fn file_name(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// A new strip of type `kind` (COLOR, TEXT, ADJUSTMENT, SCENE, IMAGE:path,
/// MOVIE:path, SOUND:path) at the current frame.
pub fn new_strip(d: &Doc, kind: &str) -> Result<Strip, String> {
    let (k, path) = kind.split_once(':').map_or((kind, ""), |(a, b)| (a, b));
    let fps = d.scene.fps.max(1.0);
    let frame = d.frame as f32;
    let len = 25.0;
    Ok(match k {
        "COLOR" => Strip::new("Color", StripKind::Color { color: [0.5, 0.5, 0.5] }, 1, frame, len),
        "TEXT" => Strip::new("Text", StripKind::Text(TextStrip::default()), 1, frame, len),
        "ADJUSTMENT" => Strip::new("Adjustment Layer", StripKind::Adjustment, 1, frame, len),
        "SCENE" => {
            let n = (d.scene.frame_end - d.scene.frame_start + 1).max(1) as f32;
            Strip::new("Scene", StripKind::Scene { scene: "Scene".into(), camera: String::new() }, 1, frame, n)
        }
        "IMAGE" => {
            if path.is_empty() {
                return Err("Image strips need a file".into());
            }
            // A directory of numbered files, or one file.
            let (dir, files) = match std::fs::read_dir(path) {
                Ok(rd) => {
                    let mut f: Vec<String> = rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.to_ascii_lowercase().ends_with(".png")).collect();
                    f.sort();
                    (path.trim_end_matches('/').to_string() + "/", f)
                }
                Err(_) => {
                    let (dir, f) = path.rsplit_once('/').map_or(("", path), |(a, b)| (a, b));
                    (if dir.is_empty() { String::new() } else { format!("{dir}/") }, vec![f.to_string()])
                }
            };
            if files.is_empty() {
                return Err(format!("No images in {path}"));
            }
            let n = if files.len() > 1 { files.len() as f32 } else { len };
            Strip::new(&file_name(path), StripKind::Image { directory: dir, files }, 1, frame, n)
        }
        "MOVIE" => Strip::new(&file_name(path), StripKind::Movie { path: path.to_string() }, 1, frame, 100.0),
        "SOUND" => {
            let bytes = std::fs::read(path).map_err(|e| format!("Cannot read {path}: {e}"))?;
            let (rate, _, frames, peaks) = wav_info(&bytes, 512).ok_or_else(|| format!("{path} is not a WAV file Polyamour can read"))?;
            let n = (frames as f32 / rate.max(1) as f32 * fps).ceil().max(1.0);
            Strip::new(&file_name(path), StripKind::Sound { path: path.to_string(), volume: 1.0, pan: 0.0, waveform: peaks }, 1, frame, n)
        }
        _ => return Err(format!("Unknown strip type '{k}'")),
    })
}

/// Sequencer operators. Returns false for names it does not know.
pub fn run(d: &mut Doc, op: &Op) -> bool {
    let id = op.id.as_str();
    if !id.starts_with("sequencer.") {
        return false;
    }
    let arg = op.arg.as_str();
    let frame = d.frame as f32;
    match id {
        "sequencer.strip_add" => match new_strip(d, arg) {
            Ok(st) => {
                d.push_undo("Add Strip");
                let i = seq(d).add(st);
                d.report = format!("Added {}", d.scene.sequence_editor.as_ref().map_or("", |q| q.strips[i].name.as_str()));
                d.touch();
            }
            Err(e) => d.report = e,
        },
        "sequencer.effect_strip_add" => {
            let Some(e) = Effect::new(arg) else {
                d.report = format!("Unknown effect '{arg}'");
                return true;
            };
            let mut sel = d.scene.sequence_editor.as_ref().map(|q| q.selected()).unwrap_or_default();
            if let Some(q) = d.scene.sequence_editor.as_ref() {
                sel.sort_by_key(|&i| (q.strips[i].channel, i));
            }
            let need = e.inputs();
            if sel.len() < need {
                d.report = format!("{} needs {need} selected strip(s)", e.label());
                return true;
            }
            d.push_undo("Add Effect Strip");
            let r = seq(d).add_effect(e, &sel, 1, frame);
            if r.is_none() {
                d.report = "Cannot add the effect here".into();
            }
            d.touch();
        }
        "sequencer.split" => {
            d.push_undo("Split Strips");
            let n = seq(d).split(frame, arg == "HARD").len();
            d.report = if n == 0 { "No selected strip under the current frame".into() } else { format!("Split {n} strip(s)") };
            d.touch();
        }
        "sequencer.duplicate" => {
            d.push_undo("Duplicate Strips");
            seq(d).duplicate();
            d.touch();
        }
        "sequencer.delete" => {
            d.push_undo("Delete Strips");
            let n = seq(d).delete();
            d.report = format!("Deleted {n} strip(s)");
            d.touch();
        }
        "sequencer.meta_make" => {
            d.push_undo("Make Meta Strip");
            if seq(d).make_meta().is_none() {
                d.report = "Select strips to make a meta strip".into();
            }
            d.touch();
        }
        "sequencer.meta_separate" => {
            d.push_undo("Separate Meta Strips");
            seq(d).separate_meta();
            d.touch();
        }
        "sequencer.mute" | "sequencer.unmute" => {
            d.push_undo(if id.ends_with("unmute") { "Unmute Strips" } else { "Mute Strips" });
            seq(d).set_mute(id == "sequencer.mute", arg == "UNSELECTED");
            d.touch();
        }
        "sequencer.lock" | "sequencer.unlock" => {
            d.push_undo("Lock Strips");
            seq(d).set_lock(id == "sequencer.lock");
            d.touch();
        }
        "sequencer.select_all" => {
            let on = match arg {
                "DESELECT" => Some(false),
                "SELECT" => Some(true),
                _ => None,
            };
            seq(d).select_all(on);
            d.touch();
        }
        "sequencer.snap" => {
            d.push_undo("Snap Strips");
            seq(d).snap(frame);
            d.touch();
        }
        "sequencer.translate" => {
            // "df,dc": frames and channels.
            let mut it = arg.split(',');
            let df = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0.0);
            let dc = it.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
            d.push_undo("Move Strips");
            seq(d).translate(df, dc);
            d.touch();
        }
        "sequencer.extend" => {
            let df = arg.parse().unwrap_or(0.0);
            d.push_undo("Extend Strips");
            let right = df >= 0.0;
            seq(d).extend(frame, df, right);
            d.touch();
        }
        "sequencer.modifier_add" => {
            let Some(m) = StripModifier::new(arg) else {
                d.report = format!("Unknown strip modifier '{arg}'");
                return true;
            };
            let Some(a) = d.scene.sequence_editor.as_ref().and_then(|q| q.active) else {
                d.report = "No active strip".into();
                return true;
            };
            d.push_undo("Add Strip Modifier");
            if let Some(st) = seq(d).strips.get_mut(a) {
                st.modifiers.push(m);
            }
            d.touch();
        }
        "sequencer.modifier_remove" => {
            let (Some(a), Ok(i)) = (d.scene.sequence_editor.as_ref().and_then(|q| q.active), arg.parse::<usize>()) else { return true };
            d.push_undo("Remove Strip Modifier");
            if let Some(st) = seq(d).strips.get_mut(a) {
                if i < st.modifiers.len() {
                    st.modifiers.remove(i);
                }
            }
            d.touch();
        }
        "sequencer.active" => {
            if let Ok(i) = arg.parse::<usize>() {
                let q = seq(d);
                if i < q.strips.len() {
                    q.active = Some(i);
                }
            }
        }
        "sequencer.use_toggle" => {
            d.push_undo("Sequencer Output");
            let q = seq(d);
            q.use_sequencer = !q.use_sequencer;
            d.touch();
        }
        _ => d.report = format!("Unknown operator {id}"),
    }
    true
}

/// `--seq-demo`: a colour backdrop, the scene over it and a title, with a
/// cross into the title card, in the Video Editing workspace.
pub fn demo(d: &mut Doc) {
    let mut q = Sequencer::default();
    let mut bg = Strip::new("Backdrop", StripKind::Color { color: [0.05, 0.08, 0.2] }, 1, 1.0, 120.0);
    bg.blend = sex_3d::sequencer::StripBlend::Replace;
    q.add(bg);
    let mut sc = Strip::new("Scene", StripKind::Scene { scene: "Scene".into(), camera: String::new() }, 2, 1.0, 80.0);
    sc.opacity = 0.9;
    q.add(sc);
    let title = TextStrip { text: "Polyamour".into(), font_size: 120.0, color: [1.0, 0.85, 0.4, 1.0], location: [0.5, 0.2], use_shadow: true, ..TextStrip::default() };
    q.add(Strip::new("Title", StripKind::Text(title), 6, 1.0, 120.0));
    let mut card = Strip::new("Card", StripKind::Color { color: [0.6, 0.15, 0.25] }, 4, 60.0, 60.0);
    card.blend = sex_3d::sequencer::StripBlend::AlphaOver;
    q.add(card);
    if let Some(m) = StripModifier::new("BRIGHT_CONTRAST") {
        q.strips[1].modifiers.push(m);
    }
    q.select_all(Some(false));
    let (a, b) = (q.find("Scene").unwrap_or(0), q.find("Card").unwrap_or(0));
    q.add_effect(Effect::Cross, &[a, b], 5, 60.0);
    q.active = q.find("Title");
    d.scene.sequence_editor = Some(q);
    d.settings.resolution_x = 960;
    d.settings.resolution_y = 540;
    d.frame = 70;
    d.want_workspace = Some("Video Editing".into());
    d.touch();
}

// ---------------------------------------------------------------- rendering

/// Text coverage at a pixel size, lines stacked (the sequencer's text strips).
pub fn text_coverage(s: &str, size: f32) -> Option<(u32, u32, Vec<f32>)> {
    let px = size.round().clamp(4.0, 600.0) as u32;
    let mut pen = sex_office::Pen::sans(px);
    let lines: Vec<&str> = s.split('\n').collect();
    let lh = pen.line_height().max(1);
    let w = lines.iter().map(|l| pen.measure(l)).max().unwrap_or(0).max(1) as usize;
    let h = (lh as usize) * lines.len();
    let mut buf = vec![0u32; w * h];
    {
        let mut c = sex_office::Canvas::new(&mut buf, w, h);
        for (i, l) in lines.iter().enumerate() {
            pen.text(&mut c, 0, i as i32 * lh, l, 0xffffff, None);
        }
    }
    Some((w as u32, h as u32, buf.iter().map(|p| (p & 0xff) as f32 / 255.0).collect()))
}

/// A render of `scene` at `frame` through camera `camera` (empty: active).
fn scene_image(base: &Scene, settings: &RenderSettings, camera: &str, frame: i32, w: u32, h: u32) -> Option<Image> {
    let mut scene = base.clone();
    if !camera.is_empty() {
        if let Some(c) = scene.objects.iter().position(|o| o.name == camera && matches!(o.data, ObjectData::Camera(_))) {
            scene.active_camera = Some(c);
        }
    }
    scene.active_camera?;
    let st = with_depth(&scene, RenderSettings { width: w, height: h, frame, ..settings.clone() });
    let mut r = sex_3d::render::render(&scene, &st);
    composite_gp(&scene, &mut r, frame);
    Some(r.color)
}

/// Ask for the Depth pass when the scene has Grease Pencil (its strokes are
/// depth tested against it).
pub fn with_depth(scene: &Scene, mut s: RenderSettings) -> RenderSettings {
    if scene.objects.iter().any(|o| matches!(o.data, ObjectData::GreasePencil(_))) {
        s.passes = s.passes.with(Pass::Depth);
    }
    s
}

/// Draw Grease Pencil over a finished render (from its camera, depth tested).
pub fn composite_gp(scene: &Scene, r: &mut RenderResult, frame: i32) -> usize {
    let depth = r.passes.get(&Pass::Depth).cloned();
    sex_3d::grease_pencil::raster::composite_render(scene, &mut r.color, depth.as_ref(), frame)
}

/// Render the sequencer at `frame` into a `w` x `h` image. `scene_at` turns
/// a frame into the scene to render for scene strips; `settings` are the
/// render settings scene strips use; `current` is an already rendered frame
/// of the scene (reused when a scene strip shows it).
pub fn render_seq(q: &Sequencer, frame: i32, w: u32, h: u32, scene_at: &dyn Fn(i32) -> Scene, settings: &RenderSettings, current: Option<(i32, &Image)>) -> Image {
    let mut images: HashMap<String, Option<Image>> = HashMap::new();
    let mut scene_cb = |_: &str, camera: &str, f: i32| -> Option<Image> {
        if let Some((cf, img)) = current {
            if cf == f && camera.is_empty() && img.width == w && img.height == h {
                return Some(img.clone());
            }
        }
        scene_image(&scene_at(f), settings, camera, f, w, h)
    };
    let mut image_cb = |path: &str| -> Option<Image> {
        images
            .entry(path.to_string())
            .or_insert_with(|| {
                let bytes = std::fs::read(path).ok()?;
                sex_3d::io::decode_image(path, &bytes).ok()
            })
            .clone()
    };
    let text = |s: &str, size: f32| text_coverage(s, size);
    let mut ctx = RenderCtx { width: w, height: h, scene: &mut scene_cb, image: &mut image_cb, text: &text };
    render_frame(&q.strips, frame as f32, &mut ctx)
}

/// After an F12 render of `scene`: composite Grease Pencil, then replace
/// the image with the sequencer's output when that is on.
pub fn post_render(scene: &Scene, frame: i32, settings: &RenderSettings, mut r: RenderResult, scene_at: &dyn Fn(i32) -> Scene) -> RenderResult {
    composite_gp(scene, &mut r, frame);
    if active(scene) {
        if let Some(q) = scene.sequence_editor.as_ref() {
            let (w, h) = (r.color.width, r.color.height);
            let img = render_seq(q, frame, w, h, scene_at, settings, Some((frame, &r.color)));
            r.color = img;
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_op(d: &mut Doc, id: &str, arg: &str) {
        d.run(&Op::with(id, arg));
    }

    #[test]
    fn strips_and_edit_ops() {
        let mut d = Doc::default();
        d.frame = 1;
        run_op(&mut d, "sequencer.strip_add", "COLOR");
        d.frame = 10;
        run_op(&mut d, "sequencer.strip_add", "TEXT");
        let q = d.scene.sequence_editor.as_ref().unwrap();
        assert_eq!(q.strips.len(), 2);
        assert_eq!(q.strips[0].channel, 1);
        // The text strip overlaps the colour strip: it goes up a channel.
        assert_eq!(q.strips[1].channel, 2);
        // Select both, add a cross between them.
        run_op(&mut d, "sequencer.select_all", "SELECT");
        run_op(&mut d, "sequencer.effect_strip_add", "CROSS");
        let q = d.scene.sequence_editor.as_ref().unwrap();
        assert_eq!(q.strips.len(), 3);
        let cross = &q.strips[2];
        assert_eq!((cross.final_start(), cross.final_end()), (10.0, 26.0));
        // Split the text strip at frame 20.
        run_op(&mut d, "sequencer.select_all", "DESELECT");
        seq(&mut d).strips[1].select = true;
        d.frame = 20;
        run_op(&mut d, "sequencer.split", "SOFT");
        let q = d.scene.sequence_editor.as_ref().unwrap();
        assert_eq!(q.strips.len(), 4);
        // Undo brings back three strips.
        run_op(&mut d, "ed.undo", "");
        assert_eq!(d.scene.sequence_editor.as_ref().unwrap().strips.len(), 3);
        // Mute and meta.
        run_op(&mut d, "sequencer.select_all", "SELECT");
        run_op(&mut d, "sequencer.mute", "");
        assert!(d.scene.sequence_editor.as_ref().unwrap().strips.iter().all(|s| s.mute));
        run_op(&mut d, "sequencer.unmute", "");
        run_op(&mut d, "sequencer.meta_make", "");
        assert_eq!(d.scene.sequence_editor.as_ref().unwrap().strips.len(), 1);
        run_op(&mut d, "sequencer.meta_separate", "");
        assert_eq!(d.scene.sequence_editor.as_ref().unwrap().strips.len(), 3);
    }

    #[test]
    fn text_strip_renders() {
        let cov = text_coverage("Hi", 40.0).unwrap();
        assert!(cov.0 > 10 && cov.1 > 10);
        assert!(cov.2.iter().any(|&a| a > 0.5));
        let mut q = Sequencer::default();
        let mut c = Strip::new("Red", StripKind::Color { color: [1.0, 0.0, 0.0] }, 1, 1.0, 10.0);
        c.select = false;
        q.add(c);
        let s = Scene::default();
        let img = render_seq(&q, 3, 8, 4, &|_| s.clone(), &RenderSettings::default(), None);
        assert_eq!(img.pixels.len(), 32);
        assert!((img.pixels[0][0] - 1.0).abs() < 1e-4 && img.pixels[0][1].abs() < 1e-4);
        // Outside the strip: transparent.
        let img = render_seq(&q, 30, 8, 4, &|_| s.clone(), &RenderSettings::default(), None);
        assert_eq!(img.pixels[0], [0.0; 4]);
    }
}
