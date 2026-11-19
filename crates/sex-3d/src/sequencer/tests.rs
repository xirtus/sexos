// SPDX-License-Identifier: GPL-3.0-only
use super::render::*;
use super::*;
use crate::scene::Image;

fn color(name: &str, c: [f32; 3], ch: u32, start: f32, len: f32) -> Strip {
    Strip::new(name, StripKind::Color { color: c }, ch, start, len)
}

fn no_text(_: &str, _: f32) -> Option<(u32, u32, Vec<f32>)> {
    None
}

fn render(strips: &[Strip], frame: f32) -> Image {
    let mut scene = |_: &str, _: &str, _: i32| -> Option<Image> { None };
    let mut image = |p: &str| -> Option<Image> {
        // "half.png": a 2x1 image, left opaque red, right transparent.
        (p == "img/half.png").then(|| Image { name: p.into(), width: 2, height: 1, pixels: vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.0]] })
    };
    let mut ctx = RenderCtx { width: 8, height: 4, scene: &mut scene, image: &mut image, text: &no_text };
    render_frame(strips, frame, &mut ctx)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn strip_timing() {
    let mut s = color("A", [1.0; 3], 1, 10.0, 20.0);
    assert_eq!((s.final_start(), s.final_end(), s.duration()), (10.0, 30.0, 20.0));
    assert!(s.shown_at(10.0) && s.shown_at(29.0) && !s.shown_at(30.0) && !s.shown_at(9.0));
    s.set_range(15.0, 25.0);
    assert_eq!((s.offset_start, s.offset_end), (5.0, 5.0));
    assert_eq!(s.content_frame(15.0), 5.0);
    s.reverse = true;
    assert_eq!(s.content_frame(10.0), 19.0);
}

#[test]
fn add_shuffles_into_free_channels_and_names_are_unique() {
    let mut q = Sequencer::default();
    let a = q.add(color("Color", [1.0; 3], 1, 0.0, 10.0));
    let b = q.add(color("Color", [1.0; 3], 1, 5.0, 10.0));
    assert_eq!(q.strips[a].channel, 1);
    assert_eq!(q.strips[b].channel, 2, "overlap goes up a channel");
    assert_ne!(q.strips[a].name, q.strips[b].name);
    let c = q.add(color("C", [1.0; 3], 1, 10.0, 5.0));
    assert_eq!(q.strips[c].channel, 1, "touching end-to-start is free");
    assert_eq!(q.range(), Some((0.0, 15.0)));
}

#[test]
fn grab_moves_and_shuffles_and_respects_locks() {
    let mut q = Sequencer::default();
    q.add(color("A", [1.0; 3], 1, 0.0, 10.0));
    let b = q.add(color("B", [1.0; 3], 1, 20.0, 10.0));
    q.select_all(Some(false));
    q.strips[b].select = true;
    assert_eq!(q.translate(-15.0, 0), 1);
    assert_eq!(q.strips[b].final_start(), 5.0);
    assert_eq!(q.strips[b].channel, 2, "overlap with A shuffles up");
    q.translate(20.0, -1);
    assert_eq!((q.strips[b].final_start(), q.strips[b].channel), (25.0, 1));
    q.set_lock(true);
    assert_eq!(q.translate(5.0, 0), 0);
    assert_eq!(q.strips[b].final_start(), 25.0);
    q.set_lock(false);
    q.channel_mut(1).lock = true;
    assert_eq!(q.translate(5.0, 0), 0, "locked channel");
}

#[test]
fn soft_and_hard_split() {
    let mut q = Sequencer::default();
    let files: Vec<String> = (0..20).map(|i| format!("f{i:02}.png")).collect();
    let a = q.add(Strip::new("Seq", StripKind::Image { directory: "img".into(), files }, 1, 0.0, 20.0));
    let made = q.split(8.0, false);
    assert_eq!(made.len(), 1);
    let r = made[0];
    assert_eq!((q.strips[a].final_start(), q.strips[a].final_end()), (0.0, 8.0));
    assert_eq!((q.strips[r].final_start(), q.strips[r].final_end()), (8.0, 20.0));
    // Soft: the content is still there behind the handle.
    assert_eq!(q.strips[a].length, 20.0);
    assert_eq!(q.strips[r].content_frame(8.0), 8.0);
    assert!(q.strips[r].select && !q.strips[a].select);
    // Hard split of the right part: content is cut, the frame mapping is kept.
    let made = q.split(12.0, true);
    let rr = made[0];
    assert_eq!(q.strips[r].length, 12.0);
    assert_eq!(q.strips[rr].start, 12.0);
    assert_eq!(q.strips[rr].length, 8.0);
    assert_eq!(q.strips[rr].content_frame(12.0), 12.0);
    assert_eq!(q.strips[rr].content_frame(19.0), 19.0);
    // A hard-cut part cannot be extended back past its content.
    q.set_handle(rr, false, 5.0);
    assert_eq!(q.strips[rr].final_start(), 12.0);
    // A soft-cut part can.
    q.set_handle(a, true, 15.0);
    assert_eq!(q.strips[a].final_end(), 15.0);
    // Nothing to split outside a strip.
    q.select_all(Some(true));
    assert!(q.split(100.0, false).is_empty());
}

#[test]
fn extend_moves_handles_and_strips() {
    let mut q = Sequencer::default();
    let a = q.add(color("A", [1.0; 3], 1, 0.0, 10.0));
    let b = q.add(color("B", [1.0; 3], 1, 20.0, 10.0));
    q.select_all(Some(true));
    q.extend(5.0, 4.0, true);
    assert_eq!(q.strips[a].final_end(), 14.0, "colour strips grow");
    assert_eq!(q.strips[b].final_start(), 24.0, "strips past the frame move");
    q.extend(5.0, -2.0, false);
    assert_eq!(q.strips[a].final_start(), -2.0, "colour strips grow at the start too");
}

#[test]
fn duplicate_delete_and_effect_inputs() {
    let mut q = Sequencer::default();
    let a = q.add(color("A", [1.0, 0.0, 0.0], 1, 0.0, 20.0));
    let b = q.add(color("B", [0.0, 0.0, 1.0], 2, 10.0, 20.0));
    let e = q.add_effect(Effect::new("CROSS").unwrap(), &[a, b], 1, 0.0).unwrap();
    assert_eq!((q.strips[e].final_start(), q.strips[e].final_end()), (10.0, 20.0), "spans the overlap");
    assert_eq!(q.strips[e].channel, 3);
    assert!(q.add_effect(Effect::new("CROSS").unwrap(), &[a], 1, 0.0).is_none());
    // Moving an input moves the effect.
    q.select_all(Some(false));
    q.strips[b].select = true;
    q.translate(-5.0, 0);
    assert_eq!(q.strips[e].final_start(), 5.0);
    // Duplicating inputs and effect re-links the copy.
    q.select_all(Some(true));
    let made = q.duplicate();
    assert_eq!(made.len(), 3);
    let copy = made.iter().copied().find(|&i| q.strips[i].kind.is_effect()).unwrap();
    for n in q.strips[copy].effect_inputs() {
        assert!(made.iter().any(|&i| &q.strips[i].name == n), "copy points at copies");
    }
    for &i in &made {
        let s = &q.strips[i];
        assert!(q.is_free(s.channel, s.final_start(), s.final_end(), &[i]));
    }
    // Deleting an input deletes the effect.
    q.select_all(Some(false));
    q.strips[a].select = true;
    assert_eq!(q.delete(), 2);
    assert_eq!(q.strips.len(), 4);
}

#[test]
fn meta_mute_snap_and_select() {
    let mut q = Sequencer::default();
    q.add(color("A", [1.0; 3], 1, 0.0, 10.0));
    q.add(color("B", [1.0; 3], 2, 5.0, 10.0));
    q.add(color("C", [1.0; 3], 1, 30.0, 5.0));
    q.select_box(0.0, 20.0, 1, 2, false);
    assert_eq!(q.selected().len(), 2);
    let m = q.make_meta().unwrap();
    assert_eq!(q.strips.len(), 2);
    assert_eq!((q.strips[m].final_start(), q.strips[m].final_end()), (0.0, 15.0));
    assert_eq!(q.strip_at(3.0, 1), Some(m));
    assert_eq!(q.separate_meta(), 1);
    assert_eq!(q.strips.len(), 3);
    q.select_pick(32.0, 1, false);
    assert_eq!(q.selected().len(), 1);
    q.set_mute(true, true);
    assert_eq!(q.strips.iter().filter(|s| s.mute).count(), 2);
    q.set_mute(false, true);
    q.snap(40.0);
    let c = q.find("C").unwrap();
    assert_eq!(q.strips[c].final_start(), 40.0);
    q.select_side(20.0, true);
    assert_eq!(q.selected(), vec![c]);
}

#[test]
fn render_stack_blend_and_opacity() {
    let red = color("R", [1.0, 0.0, 0.0], 1, 0.0, 10.0);
    let mut blue = color("B", [0.0, 0.0, 1.0], 2, 0.0, 10.0);
    blue.blend = StripBlend::AlphaOver;
    blue.opacity = 0.25;
    let img = render(&[red.clone(), blue.clone()], 1.0);
    assert!(close(img.pixels[0], [0.75, 0.0, 0.25, 1.0]), "{:?}", img.pixels[0]);
    // Channel order, not list order.
    let img = render(&[blue.clone(), red.clone()], 1.0);
    assert!(close(img.pixels[0], [0.75, 0.0, 0.25, 1.0]));
    // Multiply, Add, Screen.
    let mut grey = color("G", [0.5, 0.5, 0.5], 2, 0.0, 10.0);
    grey.blend = StripBlend::Multiply;
    let img = render(&[red.clone(), grey.clone()], 1.0);
    assert!(close(img.pixels[0], [0.5, 0.0, 0.0, 1.0]));
    grey.blend = StripBlend::Add;
    assert!(close(render(&[red.clone(), grey.clone()], 1.0).pixels[0], [1.5, 0.5, 0.5, 1.0]));
    grey.blend = StripBlend::Screen;
    assert!(close(render(&[red.clone(), grey.clone()], 1.0).pixels[0], [1.0, 0.5, 0.5, 1.0]));
    // Muted strips and out-of-range frames are not drawn.
    let mut m = blue.clone();
    m.mute = true;
    assert!(close(render(&[red.clone(), m], 1.0).pixels[0], [1.0, 0.0, 0.0, 1.0]));
    assert!(close(render(&[red], 12.0).pixels[0], [0.0; 4]));
}

#[test]
fn images_fit_and_keep_alpha() {
    let s = Strip::new("I", StripKind::Image { directory: "img".into(), files: vec!["half.png".into()] }, 1, 0.0, 5.0);
    let img = render(&[s.clone()], 0.0);
    // 2x1 fitted into 8x4: left half red, right half empty.
    assert!(close(img.pixels[4 * 8 + 1 - 8], [1.0, 0.0, 0.0, 1.0]));
    assert!(close(img.pixels[8 + 6], [0.0; 4]));
    let mut f = s;
    f.flip_x = true;
    let img = render(&[f], 0.0);
    assert!(close(img.pixels[8 + 6], [1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn adjustment_and_modifiers() {
    let grey = color("G", [0.5, 0.5, 0.5], 1, 0.0, 10.0);
    let mut adj = Strip::new("Adj", StripKind::Adjustment, 2, 0.0, 10.0);
    adj.multiply = 2.0;
    assert!(close(render(&[grey.clone(), adj.clone()], 0.0).pixels[0], [1.0, 1.0, 1.0, 1.0]));
    let mut adj = Strip::new("Adj", StripKind::Adjustment, 2, 0.0, 10.0);
    let mut m = StripModifier::new("BRIGHT_CONTRAST").unwrap();
    m.kind = ModKind::BrightContrast { bright: 10.0, contrast: 0.0 };
    adj.modifiers.push(m);
    assert!(close(render(&[grey.clone(), adj], 0.0).pixels[0], [0.6, 0.6, 0.6, 1.0]));
    // Saturation 0 makes grey.
    let mut red = color("R", [1.0, 0.0, 0.0], 1, 0.0, 10.0);
    red.saturation = 0.0;
    let p = render(&[red], 0.0).pixels[0];
    assert!((p[0] - 0.2126).abs() < 1e-3 && (p[1] - 0.2126).abs() < 1e-3);
    // Modifier formulas.
    let cb = ModKind::ColorBalance { lift: [1.0; 3], gamma: [2.0, 1.0, 1.0], gain: [1.0, 2.0, 1.0] };
    let c = apply_modifier(&cb, [0.25, 0.25, 0.25]);
    assert!((c[0] - 0.5).abs() < 1e-4 && (c[1] - 0.5).abs() < 1e-4 && (c[2] - 0.25).abs() < 1e-4);
    let cv = ModKind::Curves { points: vec![(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)] };
    assert!((apply_modifier(&cv, [0.25; 3])[0] - 0.4).abs() < 1e-4);
    let bc = ModKind::BrightContrast { bright: 0.0, contrast: 50.0 };
    let c = apply_modifier(&bc, [0.75; 3]);
    assert!((c[0] - (0.75 / 0.5 - 0.25 / 0.5)).abs() < 1e-4, "{c:?}");
    let wb = ModKind::WhiteBalance { white: [0.5, 1.0, 1.0] };
    assert_eq!(apply_modifier(&wb, [0.25; 3])[0], 0.5);
}

#[test]
fn effect_math() {
    let a: Frame = vec![[0.2, 0.4, 0.0, 1.0]; 4];
    let b: Frame = vec![[0.6, 0.0, 0.8, 1.0]; 4];
    let half: Frame = vec![[0.25, 0.25, 0.25, 0.5]; 4];
    let e = |id: &str| Effect::new(id).unwrap();
    assert!(close(effect2(&e("CROSS"), &a, &b, 0.5, 2, 2)[0], [0.4, 0.2, 0.4, 1.0]));
    let g = effect2(&e("GAMMA_CROSS"), &a, &b, 0.5, 2, 2)[0];
    let gx = ((0.2f32.sqrt() + 0.6f32.sqrt()) / 2.0).powi(2);
    assert!((g[0] - gx).abs() < 1e-4);
    assert!(close(effect2(&e("ADD"), &a, &b, 1.0, 2, 2)[0], [0.8, 0.4, 0.8, 1.0]));
    assert!(close(effect2(&e("SUBTRACT"), &a, &b, 1.0, 2, 2)[0], [0.0, 0.4, 0.0, 1.0]));
    assert!(close(effect2(&e("MULTIPLY"), &a, &b, 1.0, 2, 2)[0], [0.12, 0.0, 0.0, 1.0]));
    assert!(close(effect2(&e("ALPHA_OVER"), &a, &half, 1.0, 2, 2)[0], [0.35, 0.45, 0.25, 1.0]));
    assert!(close(effect2(&e("ALPHA_UNDER"), &half, &a, 1.0, 2, 2)[0], [0.35, 0.45, 0.25, 1.0]));
    let mix = Effect::ColorMix { blend: StripBlend::Darken, factor: 1.0 };
    assert!(close(effect2(&mix, &a, &b, 1.0, 2, 2)[0], [0.2, 0.0, 0.0, 1.0]));
    // A single wipe at 0 shows A, at 1 shows B.
    let w = e("WIPE");
    assert!(close(effect2(&w, &a, &b, 0.0, 2, 2)[0], a[0]));
    assert!(close(effect2(&w, &a, &b, 1.0, 2, 2)[0], b[0]));
    // Halfway: the left column is past the edge, the right is not.
    let big_a: Frame = vec![[0.0, 0.0, 0.0, 1.0]; 64];
    let big_b: Frame = vec![[1.0, 1.0, 1.0, 1.0]; 64];
    let m = effect2(&Effect::Wipe { kind: WipeType::Single, angle: 0.0, blur: 0.0, wipe_in: false }, &big_a, &big_b, 0.5, 8, 8);
    assert!(m[0][0] > 0.9 && m[7][0] < 0.1, "{:?} {:?}", m[0], m[7]);
    // Every effect id round-trips.
    for id in Effect::IDS {
        assert_eq!(Effect::new(id).unwrap().id(), id);
    }
}

#[test]
fn effect_strips_render_their_inputs() {
    let mut q = Sequencer::default();
    let a = q.add(color("A", [1.0, 0.0, 0.0], 1, 0.0, 11.0));
    let b = q.add(color("B", [0.0, 0.0, 1.0], 2, 0.0, 11.0));
    q.add_effect(Effect::new("CROSS").unwrap(), &[a, b], 3, 0.0).unwrap();
    // The fader follows the strip: 0 at its start, 1 at its end.
    assert_eq!(rendered_strips(&q.strips, 0.0).len(), 1, "inputs are drawn by the effect");
    assert!(close(render(&q.strips, 0.0).pixels[0], [1.0, 0.0, 0.0, 1.0]));
    let mid = render(&q.strips, 5.5).pixels[0];
    assert!((mid[0] - 0.5).abs() < 1e-3 && (mid[2] - 0.5).abs() < 1e-3, "{mid:?}");
    // One-input effects.
    let mut q = Sequencer::default();
    let a = q.add(color("A", [0.5, 0.5, 0.5], 1, 0.0, 10.0));
    let t = q.add_effect(Effect::Transform { translate: [4.0, 0.0], scale: [1.0, 1.0], rotation: 0.0 }, &[a], 2, 0.0).unwrap();
    let img = render(&q.strips, 1.0);
    assert!(close(img.pixels[0], [0.0; 4]) && close(img.pixels[7], [0.5, 0.5, 0.5, 1.0]), "moved right by 4");
    q.strips[t].kind = StripKind::Effect { effect: Effect::GaussianBlur { size: [2.0, 2.0] }, inputs: vec!["A".into()], fader: None };
    let p = render(&q.strips, 1.0).pixels[10];
    assert!(p[3] > 0.0 && (p[0] / p[3] - 0.5).abs() < 1e-3, "blurring a flat colour keeps its colour: {p:?}");
    q.strips[t].kind = StripKind::Effect { effect: Effect::Glow { threshold: 0.2, clamp: 1.0, boost: 1.0, blur: 1.0, only_boost: false }, inputs: vec!["A".into()], fader: None };
    assert!(render(&q.strips, 1.0).pixels[10][0] > 0.5, "glow brightens");
}

#[test]
fn speed_effect_retimes() {
    let files: Vec<String> = (0..10).map(|i| format!("f{i}.png")).collect();
    let mut q = Sequencer::default();
    let a = q.add(Strip::new("A", StripKind::Image { directory: "d".into(), files }, 1, 0.0, 10.0));
    let sp = q.add_effect(Effect::Speed { mode: SpeedMode::Multiply, factor: 2.0 }, &[a], 2, 0.0).unwrap();
    let mut seen = Vec::new();
    let mut scene = |_: &str, _: &str, _: i32| -> Option<Image> { None };
    let mut image = |p: &str| -> Option<Image> {
        seen.push(p.to_string());
        Some(Image { name: p.into(), width: 1, height: 1, pixels: vec![[1.0; 4]] })
    };
    let mut ctx = RenderCtx { width: 2, height: 2, scene: &mut scene, image: &mut image, text: &no_text };
    render_frame(&q.strips, 3.0, &mut ctx);
    assert_eq!(seen.last().map(String::as_str), Some("d/f6.png"));
    assert_eq!(q.strips[sp].duration(), 10.0);
}

#[test]
fn text_meta_and_movie_placeholder() {
    let t = Strip::new("T", StripKind::Text(TextStrip { text: "Hi".into(), font_size: 1080.0 * 0.5, ..Default::default() }), 1, 0.0, 10.0);
    let img = render(&[t], 0.0);
    assert!(img.pixels.iter().any(|p| p[3] > 0.9), "block text drawn");
    assert!(img.pixels.iter().any(|p| p[3] == 0.0));
    let mut q = Sequencer::default();
    q.add(color("A", [0.0, 1.0, 0.0], 1, 0.0, 10.0));
    q.select_all(Some(true));
    q.make_meta();
    assert!(close(render(&q.strips, 2.0).pixels[0], [0.0, 1.0, 0.0, 1.0]));
    let mv = Strip::new("M", StripKind::Movie { path: "x.mp4".into() }, 1, 0.0, 10.0);
    assert_eq!(render(&[mv], 0.0).pixels[0][3], 1.0);
}

fn wav(bits: u16, channels: u16, samples: &[i16]) -> Vec<u8> {
    let mut data = Vec::new();
    for &s in samples {
        match bits {
            16 => data.extend_from_slice(&s.to_le_bytes()),
            8 => data.push(((s as i32 >> 8) + 128) as u8),
            _ => unreachable!(),
        }
    }
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&8000u32.to_le_bytes());
    v.extend_from_slice(&(8000 * channels as u32 * bits as u32 / 8).to_le_bytes());
    v.extend_from_slice(&(channels * bits / 8).to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&(data.len() as u32).to_le_bytes());
    v.extend(data);
    v
}

#[test]
fn wav_metadata_and_waveform() {
    let samples: Vec<i16> = (0..800).map(|i| if i < 400 { 16384 } else { -32767 }).collect();
    let (rate, ch, frames, wave) = wav_info(&wav(16, 2, &samples), 4).unwrap();
    assert_eq!((rate, ch, frames), (8000, 2, 400));
    assert!((wave[0] - 0.5).abs() < 1e-3 && (wave[3] - 1.0).abs() < 1e-3, "{wave:?}");
    let (_, _, frames, wave) = wav_info(&wav(8, 1, &samples), 2).unwrap();
    assert_eq!(frames, 800);
    assert!((wave[0] - 0.5).abs() < 0.02);
    assert!(wav_info(b"RIFF....AVI ", 4).is_none());
}

#[test]
fn ids_and_labels() {
    for b in StripBlend::ALL {
        assert_eq!(StripBlend::from_id(&b.id()), Some(b));
    }
    for id in ["COLOR_BALANCE", "CURVES", "BRIGHT_CONTRAST", "WHITE_BALANCE"] {
        assert_eq!(StripModifier::new(id).unwrap().type_id(), id);
    }
    let q = Sequencer::default();
    assert!(q.use_sequencer && !q.has_visual_strips());
    assert_eq!(q.channel(3).name, "Channel 3");
}
