// SPDX-License-Identifier: GPL-3.0-only
//! Rendering a sequencer frame: the strips shown at the frame, bottom
//! channel first (effect inputs are drawn by their effects only), each
//! turned into an image (colour, image file, scene, text, adjustment of
//! what is below, effect, meta), colour corrected, then blended with its
//! blend mode and opacity. Images are linear RGBA, premultiplied.
use super::{Effect, ModKind, SpeedMode, Strip, StripBlend, StripKind, StripModifier, TextAlign, WipeType};
use crate::scene::Image;

/// What rendering needs from the outside world.
pub struct RenderCtx<'a> {
    pub width: u32,
    pub height: u32,
    /// The scene (named) through its camera (named, empty: the active one) at a frame.
    pub scene: &'a mut dyn FnMut(&str, &str, i32) -> Option<Image>,
    /// An image file (directory joined with the file name); callers cache.
    pub image: &'a mut dyn FnMut(&str) -> Option<Image>,
    /// Text coverage at a pixel size: (width, height, alpha per pixel, top-left first).
    pub text: &'a dyn Fn(&str, f32) -> Option<(u32, u32, Vec<f32>)>,
}

/// A premultiplied frame.
pub type Frame = Vec<[f32; 4]>;

fn blank(ctx: &RenderCtx<'_>) -> Frame {
    vec![[0.0; 4]; (ctx.width * ctx.height) as usize]
}

/// Render the sequencer at `frame` into an image.
pub fn render_frame(strips: &[Strip], frame: f32, ctx: &mut RenderCtx<'_>) -> Image {
    let px = render_stack(strips, frame, ctx);
    Image { name: "Sequencer".into(), width: ctx.width, height: ctx.height, pixels: px }
}

/// The strips composited at `frame`, in render order.
pub fn rendered_strips(strips: &[Strip], frame: f32) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..strips.len()).filter(|&i| !strips[i].mute && strips[i].shown_at(frame) && !matches!(strips[i].kind, StripKind::Sound { .. })).collect();
    // Inputs of a shown effect are rendered by that effect only.
    let inputs: Vec<String> = shown.iter().flat_map(|&i| strips[i].effect_inputs().to_vec()).collect();
    shown.retain(|&i| !inputs.contains(&strips[i].name));
    shown.sort_by_key(|&i| (strips[i].channel, i));
    shown
}

fn render_stack(strips: &[Strip], frame: f32, ctx: &mut RenderCtx<'_>) -> Frame {
    let mut acc = blank(ctx);
    for i in rendered_strips(strips, frame) {
        let s = &strips[i];
        let img = match &s.kind {
            StripKind::Adjustment => Some(acc.clone()),
            _ => strip_image(strips, s, frame, ctx, 0),
        };
        let Some(mut img) = img else { continue };
        correct(s, &mut img);
        blend(&mut acc, &img, s.blend, s.opacity.clamp(0.0, 1.0));
    }
    acc
}

/// One strip's image at `frame` (before its blend).
fn strip_image(strips: &[Strip], s: &Strip, frame: f32, ctx: &mut RenderCtx<'_>, depth: u32) -> Option<Frame> {
    if depth > 16 {
        return None;
    }
    let (w, h) = (ctx.width as usize, ctx.height as usize);
    match &s.kind {
        StripKind::Color { color } => Some(vec![[color[0], color[1], color[2], 1.0]; w * h]),
        StripKind::Image { directory, files } => {
            if files.is_empty() {
                return None;
            }
            let k = (s.content_frame(frame).floor().max(0.0) as usize).min(files.len() - 1);
            let path = if directory.is_empty() { files[k].clone() } else { format!("{}/{}", directory.trim_end_matches('/'), files[k]) };
            let img = (ctx.image)(&path)?;
            Some(place(&img, s, w, h))
        }
        StripKind::Movie { .. } => {
            // No decoder: a dark checker so the strip is visible in the preview.
            let mut f = vec![[0.0; 4]; w * h];
            for y in 0..h {
                for x in 0..w {
                    let c = if ((x / 32) + (y / 32)) % 2 == 0 { 0.08 } else { 0.12 };
                    f[y * w + x] = [c, c, c, 1.0];
                }
            }
            Some(f)
        }
        StripKind::Sound { .. } => None,
        StripKind::Scene { scene, camera } => {
            let f = (s.content_frame(frame) + 1.0).round() as i32;
            let img = (ctx.scene)(scene, camera, f)?;
            Some(place(&img, s, w, h))
        }
        StripKind::Text(t) => Some(text_image(t, ctx)),
        StripKind::Adjustment => None,
        // Children keep their own timing (they sit at their frames).
        StripKind::Meta { strips: inner } => Some(render_stack(inner, frame, ctx)),
        StripKind::Effect { effect, inputs, fader } => {
            let input = |ctx: &mut RenderCtx<'_>, k: usize, f: f32| -> Frame {
                inputs
                    .get(k)
                    .and_then(|n| strips.iter().find(|x| &x.name == n))
                    .and_then(|x| {
                        let mut img = strip_image(strips, x, f, ctx, depth + 1)?;
                        correct(x, &mut img);
                        Some(img)
                    })
                    .unwrap_or_else(|| blank(ctx))
            };
            let dur = s.duration().max(1.0);
            let fac = fader.unwrap_or(((frame - s.final_start()) / dur).clamp(0.0, 1.0));
            Some(match effect {
                Effect::Speed { mode, factor } => {
                    let src = inputs.first().and_then(|n| strips.iter().find(|x| &x.name == n));
                    let f = match (mode, src) {
                        (SpeedMode::Stretch, Some(x)) => x.final_start() + (frame - s.final_start()) / dur * x.duration(),
                        (SpeedMode::Multiply, Some(x)) => x.final_start() + (frame - s.final_start()) * factor,
                        (SpeedMode::FrameNumber, Some(x)) => x.final_start() + factor,
                        (_, None) => frame,
                    };
                    input(ctx, 0, f)
                }
                Effect::Transform { translate, scale, rotation } => {
                    let a = input(ctx, 0, frame);
                    transform(&a, w, h, *translate, *scale, *rotation)
                }
                Effect::GaussianBlur { size } => {
                    let mut a = input(ctx, 0, frame);
                    crate::grease_pencil::effects::blur(&mut a, w, h, size[0], size[1]);
                    a
                }
                Effect::Glow { threshold, clamp, boost, blur, only_boost } => {
                    let a = input(ctx, 0, frame);
                    glow(&a, w, h, *threshold, *clamp, *boost, *blur, *only_boost)
                }
                _ => {
                    let a = input(ctx, 0, frame);
                    let b = input(ctx, 1, frame);
                    effect2(effect, &a, &b, fac, w, h)
                }
            })
        }
    }
}

/// Fit an image into the frame (keeping its aspect), then the strip's
/// transform, crop and flips.
fn place(img: &Image, s: &Strip, w: usize, h: usize) -> Frame {
    let mut out = vec![[0.0; 4]; w * h];
    if img.width == 0 || img.height == 0 {
        return out;
    }
    let (iw, ih) = (img.width as f32, img.height as f32);
    let fit = (w as f32 / iw).min(h as f32 / ih);
    let t = &s.transform;
    let (sx, sy) = (fit * t.scale[0], fit * t.scale[1]);
    let (cx, cy) = (w as f32 * 0.5 + t.offset[0], h as f32 * 0.5 - t.offset[1]);
    let (piv_x, piv_y) = (iw * t.origin[0], ih * (1.0 - t.origin[1]));
    let (sn, cs) = t.rotation.sin_cos();
    let [cl, cr, cb, ct] = s.crop.map(|v| v as f32);
    for y in 0..h {
        for x in 0..w {
            // Inverse transform: frame pixel -> image pixel.
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let (rx, ry) = (dx * cs - dy * sn, dx * sn + dy * cs);
            let (mut u, mut v) = (rx / sx + piv_x, ry / sy + piv_y);
            if s.flip_x {
                u = iw - u;
            }
            if s.flip_y {
                v = ih - v;
            }
            if u < cl || u >= iw - cr || v < ct || v >= ih - cb {
                continue;
            }
            let p = img.pixels[(v as usize).min(img.height as usize - 1) * img.width as usize + (u as usize).min(img.width as usize - 1)];
            // Images are straight alpha.
            out[y * w + x] = [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
        }
    }
    out
}

fn transform(a: &Frame, w: usize, h: usize, tr: [f32; 2], sc: [f32; 2], rot: f32) -> Frame {
    let mut out = vec![[0.0; 4]; w * h];
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let (sn, cs) = rot.sin_cos();
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx - tr[0], y as f32 + 0.5 - cy + tr[1]);
            let (rx, ry) = (dx * cs - dy * sn, dx * sn + dy * cs);
            let (u, v) = (rx / sc[0].max(1e-4) + cx, ry / sc[1].max(1e-4) + cy);
            if u >= 0.0 && v >= 0.0 && (u as usize) < w && (v as usize) < h {
                out[y * w + x] = a[v as usize * w + u as usize];
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn glow(a: &Frame, w: usize, h: usize, threshold: f32, clamp: f32, boost: f32, blur: f32, only_boost: bool) -> Frame {
    let mut g: Frame = a
        .iter()
        .map(|p| {
            let m = p[0].max(p[1]).max(p[2]);
            if m > threshold {
                let k = ((m - threshold) / m.max(1e-6)).min(clamp.max(0.0));
                [p[0] * k, p[1] * k, p[2] * k, p[3] * k]
            } else {
                [0.0; 4]
            }
        })
        .collect();
    crate::grease_pencil::effects::blur(&mut g, w, h, blur * 3.0, blur * 3.0);
    a.iter()
        .zip(&g)
        .map(|(p, q)| {
            let q = q.map(|v| v * boost * 4.0);
            if only_boost {
                q
            } else {
                [p[0] + q[0], p[1] + q[1], p[2] + q[2], (p[3] + q[3]).min(1.0)]
            }
        })
        .collect()
}

/// The two-input effects at factor `f`.
pub fn effect2(e: &Effect, a: &Frame, b: &Frame, f: f32, w: usize, h: usize) -> Frame {
    let mut out = a.clone();
    for (i, o) in out.iter_mut().enumerate() {
        let (pa, pb) = (a[i], b[i]);
        *o = match e {
            Effect::Cross => lerp(pa, pb, f),
            Effect::GammaCross => {
                let g = |x: f32, y: f32| ((x.max(0.0).sqrt()) * (1.0 - f) + (y.max(0.0).sqrt()) * f).powi(2);
                [g(pa[0], pb[0]), g(pa[1], pb[1]), g(pa[2], pb[2]), pa[3] + (pb[3] - pa[3]) * f]
            }
            Effect::Add => [pa[0] + pb[0] * f, pa[1] + pb[1] * f, pa[2] + pb[2] * f, pa[3]],
            Effect::Subtract => [(pa[0] - pb[0] * f).max(0.0), (pa[1] - pb[1] * f).max(0.0), (pa[2] - pb[2] * f).max(0.0), pa[3]],
            Effect::Multiply => [pa[0] * (1.0 - f + pb[0] * f), pa[1] * (1.0 - f + pb[1] * f), pa[2] * (1.0 - f + pb[2] * f), pa[3]],
            Effect::AlphaOver => {
                let pb = pb.map(|v| v * f);
                let k = 1.0 - pb[3];
                [pb[0] + pa[0] * k, pb[1] + pa[1] * k, pb[2] + pa[2] * k, pb[3] + pa[3] * k]
            }
            Effect::AlphaUnder => {
                let pb = pb.map(|v| v * f);
                let k = 1.0 - pa[3];
                [pa[0] + pb[0] * k, pa[1] + pb[1] * k, pa[2] + pb[2] * k, pa[3] + pb[3] * k]
            }
            Effect::ColorMix { blend, factor } => mix_blend(pa, pb, *blend, *factor),
            Effect::Wipe { kind, angle, blur, wipe_in } => {
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                let k = wipe(*kind, *angle, *blur, *wipe_in, f, x / w as f32, 1.0 - y / h as f32);
                lerp(pa, pb, k)
            }
            _ => pa,
        };
    }
    out
}

/// How much of the second input shows at (u, v) (0..1, v up) for a wipe at `f`.
fn wipe(kind: WipeType, angle: f32, blur: f32, wipe_in: bool, f: f32, u: f32, v: f32) -> f32 {
    let soft = (blur * 0.5).max(1e-4);
    let edge = |d: f32| (0.5 - d / soft * 0.5).clamp(0.0, 1.0);
    let f = if wipe_in { 1.0 - f } else { f };
    let (du, dv) = (u - 0.5, v - 0.5);
    let k = match kind {
        WipeType::Single => {
            let (s, c) = angle.sin_cos();
            // Position along the wipe direction, 0..1 across the frame.
            let t = (du * c + dv * s) / (c.abs() * 0.5 + s.abs() * 0.5).max(1e-4) * 0.5 + 0.5;
            edge(t - f)
        }
        WipeType::Double => {
            let (s, c) = angle.sin_cos();
            let t = ((du * c + dv * s).abs() * 2.0) / (c.abs() + s.abs()).max(1e-4);
            edge(t - f)
        }
        WipeType::Iris => {
            let r = (du * du + dv * dv).sqrt() / std::f32::consts::FRAC_1_SQRT_2;
            edge(r - f)
        }
        WipeType::Clock => {
            let a = (dv.atan2(du) - angle + std::f32::consts::TAU * 2.0) % std::f32::consts::TAU / std::f32::consts::TAU;
            edge(a - f)
        }
    };
    if wipe_in { 1.0 - k } else { k }
}

fn lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// Per-channel blend of straight colours.
fn blend_ch(m: StripBlend, d: f32, s: f32) -> f32 {
    match m {
        StripBlend::Add => d + s,
        StripBlend::Subtract => (d - s).max(0.0),
        StripBlend::Multiply => d * s,
        StripBlend::Screen => 1.0 - (1.0 - d) * (1.0 - s),
        StripBlend::Overlay => {
            if d < 0.5 { 2.0 * d * s } else { 1.0 - 2.0 * (1.0 - d) * (1.0 - s) }
        }
        StripBlend::HardLight => {
            if s < 0.5 { 2.0 * d * s } else { 1.0 - 2.0 * (1.0 - d) * (1.0 - s) }
        }
        StripBlend::SoftLight => (1.0 - 2.0 * s) * d * d + 2.0 * s * d,
        StripBlend::Darken => d.min(s),
        StripBlend::Lighten => d.max(s),
        StripBlend::Difference => (d - s).abs(),
        StripBlend::Exclusion => d + s - 2.0 * d * s,
        StripBlend::Dodge => {
            if s >= 1.0 { 1.0 } else { (d / (1.0 - s)).min(1.0) }
        }
        StripBlend::Burn => {
            if s <= 0.0 { 0.0 } else { 1.0 - ((1.0 - d) / s).min(1.0) }
        }
        _ => s,
    }
}

/// Blend premultiplied `s` onto premultiplied `d` with mode `m` and factor `f`.
fn mix_blend(d: [f32; 4], s: [f32; 4], m: StripBlend, f: f32) -> [f32; 4] {
    match m {
        StripBlend::Replace => lerp(d, s, f),
        StripBlend::Cross => lerp(d, s, f),
        StripBlend::AlphaOver => {
            let s = s.map(|v| v * f);
            let k = 1.0 - s[3];
            [s[0] + d[0] * k, s[1] + d[1] * k, s[2] + d[2] * k, s[3] + d[3] * k]
        }
        StripBlend::AlphaUnder => {
            let s = s.map(|v| v * f);
            let k = 1.0 - d[3];
            [d[0] + s[0] * k, d[1] + s[1] * k, d[2] + s[2] * k, d[3] + s[3] * k]
        }
        _ => {
            let sa = s[3] * f;
            if sa <= 0.0 {
                return d;
            }
            let sc = [s[0] / s[3].max(1e-6), s[1] / s[3].max(1e-6), s[2] / s[3].max(1e-6)];
            let da = d[3];
            let dc = [d[0] / da.max(1e-6), d[1] / da.max(1e-6), d[2] / da.max(1e-6)];
            let mixed = [blend_ch(m, dc[0], sc[0]), blend_ch(m, dc[1], sc[1]), blend_ch(m, dc[2], sc[2])];
            // Over empty background the strip shows as it is.
            let c = [0, 1, 2].map(|k| sc[k] + (mixed[k] - sc[k]) * da.min(1.0));
            let k = 1.0 - sa;
            [c[0] * sa + d[0] * k, c[1] * sa + d[1] * k, c[2] * sa + d[2] * k, sa + d[3] * k]
        }
    }
}

/// Blend a strip's frame onto the accumulated frame.
pub fn blend(acc: &mut Frame, img: &Frame, m: StripBlend, opacity: f32) {
    for (d, s) in acc.iter_mut().zip(img) {
        *d = mix_blend(*d, *s, m, opacity);
    }
}

/// Saturation, multiply and the strip's modifiers.
fn correct(s: &Strip, img: &mut Frame) {
    let active: Vec<&StripModifier> = s.modifiers.iter().filter(|m| !m.mute).collect();
    if s.saturation == 1.0 && s.multiply == 1.0 && active.is_empty() {
        return;
    }
    for p in img.iter_mut() {
        let a = p[3];
        if a <= 0.0 {
            continue;
        }
        let mut c = [p[0] / a, p[1] / a, p[2] / a];
        if s.saturation != 1.0 {
            let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            c = c.map(|v| l + (v - l) * s.saturation);
        }
        c = c.map(|v| v * s.multiply);
        for m in &active {
            c = apply_modifier(&m.kind, c);
        }
        *p = [c[0] * a, c[1] * a, c[2] * a, a];
    }
}

/// One strip modifier on a straight linear colour.
pub fn apply_modifier(m: &ModKind, c: [f32; 3]) -> [f32; 3] {
    match m {
        // Lift/Gamma/Gain (Blender's default method): out = (gain * (in + (lift - 1) * (1 - in)))^(1/gamma).
        ModKind::ColorBalance { lift, gamma, gain } => [0, 1, 2].map(|k| {
            let v = c[k] + (lift[k] - 1.0) * (1.0 - c[k]);
            (v * gain[k]).max(0.0).powf(1.0 / gamma[k].max(1e-3))
        }),
        ModKind::Curves { points } => c.map(|v| curve(points, v)),
        ModKind::BrightContrast { bright, contrast } => {
            // Blender's brightness/contrast: out = a * in + b.
            let brightness = bright / 100.0;
            let delta = contrast / 200.0;
            let (a, b) = if *contrast > 0.0 {
                let a = 1.0 / (1.0 - delta * 2.0).max(1e-6);
                (a, a * (brightness - delta))
            } else {
                let delta = -delta;
                let a = (1.0 - delta * 2.0).max(0.0);
                (a, a * brightness + delta)
            };
            c.map(|v| v * a + b)
        }
        ModKind::WhiteBalance { white } => [0, 1, 2].map(|k| c[k] / white[k].max(1e-3)),
    }
}

/// Piecewise linear curve through sorted (x, y) points.
pub fn curve(points: &[(f32, f32)], x: f32) -> f32 {
    match points {
        [] => x,
        [p] => p.1,
        _ => {
            if x <= points[0].0 {
                return points[0].1;
            }
            for w in points.windows(2) {
                if x <= w[1].0 {
                    let t = (x - w[0].0) / (w[1].0 - w[0].0).max(1e-6);
                    return w[0].1 + (w[1].1 - w[0].1) * t;
                }
            }
            points[points.len() - 1].1
        }
    }
}

/// A text strip's image.
fn text_image(t: &super::TextStrip, ctx: &RenderCtx<'_>) -> Frame {
    let (w, h) = (ctx.width as usize, ctx.height as usize);
    let mut out = vec![[0.0; 4]; w * h];
    let size = t.font_size * h as f32 / 1080.0;
    let (tw, th, cov) = (ctx.text)(&t.text, size).unwrap_or_else(|| block_text(&t.text, size));
    let (tw, th) = (tw as i32, th as i32);
    let ax = t.location[0] * w as f32;
    let ay = (1.0 - t.location[1]) * h as f32;
    let x0 = match t.align_x {
        TextAlign::Left => ax,
        TextAlign::Center => ax - tw as f32 * 0.5,
        TextAlign::Right => ax - tw as f32,
    } as i32;
    let y0 = match t.align_y {
        TextAlign::Left => ay - th as f32,
        TextAlign::Center => ay - th as f32 * 0.5,
        TextAlign::Right => ay,
    } as i32;
    let mut paint = |ox: i32, oy: i32, col: [f32; 4], box_fill: bool| {
        let m = if box_fill { (t.box_margin * w as f32) as i32 } else { 0 };
        for y in -m..th + m {
            for x in -m..tw + m {
                let (px, py) = (x0 + ox + x, y0 + oy + y);
                if px < 0 || py < 0 || px >= w as i32 || py >= h as i32 {
                    continue;
                }
                let a = if box_fill { 1.0 } else if x >= 0 && y >= 0 && x < tw && y < th { cov[(y * tw + x) as usize] } else { 0.0 } * col[3];
                if a <= 0.0 {
                    continue;
                }
                let d = &mut out[py as usize * w + px as usize];
                let k = 1.0 - a;
                *d = [col[0] * a + d[0] * k, col[1] * a + d[1] * k, col[2] * a + d[2] * k, a + d[3] * k];
            }
        }
    };
    if t.use_box {
        paint(0, 0, t.box_color, true);
    }
    if t.use_shadow {
        let o = (size * 0.05).max(1.0) as i32;
        paint(o, o, t.shadow_color, false);
    }
    paint(0, 0, t.color, false);
    out
}

/// Fallback text: a solid block per character (when no font is supplied).
fn block_text(s: &str, size: f32) -> (u32, u32, Vec<f32>) {
    let n = s.chars().count().max(1) as u32;
    let cw = (size * 0.6).max(1.0) as u32;
    let h = size.max(1.0) as u32;
    let w = n * cw;
    let mut cov = vec![0.0; (w * h) as usize];
    for (i, ch) in s.chars().enumerate() {
        if ch.is_whitespace() {
            continue;
        }
        for y in h / 5..h * 4 / 5 {
            for x in i as u32 * cw + cw / 8..(i as u32 + 1) * cw - cw / 8 {
                cov[(y * w + x) as usize] = 1.0;
            }
        }
    }
    (w, h, cov)
}
