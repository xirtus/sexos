// SPDX-License-Identifier: GPL-3.0-only
//! Grease Pencil visual effects (Blender's "shader effects"): 2D post
//! processing on an object's drawn layer, in the viewport and in renders.
use super::raster::Canvas;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorizeMode {
    #[default]
    GrayScale,
    Sepia,
    Duotone,
    Transparent,
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GlowMode {
    #[default]
    Luminance,
    Color,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EffectKind {
    Blur { size: [f32; 2], samples: u32, rotation: f32 },
    Colorize { mode: ColorizeMode, low_color: [f32; 4], high_color: [f32; 4], factor: f32 },
    Flip { horizontal: bool, vertical: bool },
    Glow { mode: GlowMode, threshold: f32, select_color: [f32; 3], glow_color: [f32; 3], size: [f32; 2], opacity: f32, use_glow_under: bool },
    Pixelate { size: [u32; 2] },
    Rim { offset: [i32; 2], rim_color: [f32; 3], mask_color: [f32; 3], blur: [f32; 2] },
    Shadow { offset: [i32; 2], scale: [f32; 2], rotation: f32, color: [f32; 4], blur: [f32; 2] },
    Swirl { radius: f32, angle: f32 },
    Wave { orientation_vertical: bool, amplitude: f32, period: f32, phase: f32 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpEffect {
    pub name: String,
    pub show_viewport: bool,
    pub show_render: bool,
    pub kind: EffectKind,
}

pub const TYPES: [(&str, &str); 9] = [("FX_BLUR", "Blur"), ("FX_COLORIZE", "Colorize"), ("FX_FLIP", "Flip"), ("FX_GLOW", "Glow"), ("FX_PIXEL", "Pixelate"), ("FX_RIM", "Rim"), ("FX_SHADOW", "Shadow"), ("FX_SWIRL", "Swirl"), ("FX_WAVE", "Wave Distortion")];

impl GpEffect {
    pub fn new(id: &str) -> Option<GpEffect> {
        let id = id.to_ascii_uppercase();
        let kind = match id.as_str() {
            "FX_BLUR" | "BLUR" => EffectKind::Blur { size: [5.0, 5.0], samples: 8, rotation: 0.0 },
            "FX_COLORIZE" | "COLORIZE" => EffectKind::Colorize { mode: ColorizeMode::GrayScale, low_color: [0.0, 0.0, 0.0, 1.0], high_color: [1.0; 4], factor: 0.5 },
            "FX_FLIP" | "FLIP" => EffectKind::Flip { horizontal: true, vertical: false },
            "FX_GLOW" | "GLOW" => EffectKind::Glow { mode: GlowMode::Luminance, threshold: 0.1, select_color: [0.0; 3], glow_color: [0.75, 1.0, 1.0], size: [50.0, 50.0], opacity: 1.0, use_glow_under: false },
            "FX_PIXEL" | "PIXELATE" => EffectKind::Pixelate { size: [50, 50] },
            "FX_RIM" | "RIM" => EffectKind::Rim { offset: [50, -100], rim_color: [1.0, 0.6, 0.0], mask_color: [0.0; 3], blur: [0.0, 0.0] },
            "FX_SHADOW" | "SHADOW" => EffectKind::Shadow { offset: [20, 20], scale: [1.0, 1.0], rotation: 0.0, color: [0.0, 0.0, 0.0, 0.8], blur: [0.0, 0.0] },
            "FX_SWIRL" | "SWIRL" => EffectKind::Swirl { radius: 100.0, angle: std::f32::consts::FRAC_PI_2 },
            "FX_WAVE" | "WAVE" => EffectKind::Wave { orientation_vertical: false, amplitude: 10.0, period: 20.0, phase: 0.0 },
            _ => return None,
        };
        let e = GpEffect { name: String::new(), show_viewport: true, show_render: true, kind };
        Some(GpEffect { name: e.label().into(), ..e })
    }
    pub fn type_id(&self) -> &'static str {
        match self.kind {
            EffectKind::Blur { .. } => "FX_BLUR",
            EffectKind::Colorize { .. } => "FX_COLORIZE",
            EffectKind::Flip { .. } => "FX_FLIP",
            EffectKind::Glow { .. } => "FX_GLOW",
            EffectKind::Pixelate { .. } => "FX_PIXEL",
            EffectKind::Rim { .. } => "FX_RIM",
            EffectKind::Shadow { .. } => "FX_SHADOW",
            EffectKind::Swirl { .. } => "FX_SWIRL",
            EffectKind::Wave { .. } => "FX_WAVE",
        }
    }
    pub fn label(&self) -> &'static str {
        let id = self.type_id();
        TYPES.iter().find(|t| t.0 == id).map_or("Effect", |t| t.1)
    }
}

/// Bilinear sample of a premultiplied canvas (transparent outside).
fn sample(c: &[[f32; 4]], w: usize, h: usize, x: f32, y: f32) -> [f32; 4] {
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xi: f32, yi: f32| -> [f32; 4] {
        if xi < 0.0 || yi < 0.0 || xi >= w as f32 || yi >= h as f32 {
            [0.0; 4]
        } else {
            c[yi as usize * w + xi as usize]
        }
    };
    let (a, b, cc, d) = (at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0));
    let mut o = [0.0; 4];
    for i in 0..4 {
        o[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (cc[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    o
}

/// Separable box-ish blur (three passes approximate a Gaussian).
pub fn blur(px: &mut [[f32; 4]], w: usize, h: usize, rx: f32, ry: f32) {
    let pass = |px: &mut [[f32; 4]], r: usize, horizontal: bool| {
        if r == 0 {
            return;
        }
        let (n, m) = if horizontal { (w, h) } else { (h, w) };
        let mut line = vec![[0.0f32; 4]; n];
        for j in 0..m {
            let idx = |i: usize| if horizontal { j * w + i } else { i * w + j };
            let mut acc = [0.0f32; 4];
            let get = |px: &[[f32; 4]], i: i64| if i < 0 || i >= n as i64 { [0.0; 4] } else { px[idx(i as usize)] };
            for i in -(r as i64)..=(r as i64) {
                let v = get(px, i);
                for k in 0..4 {
                    acc[k] += v[k];
                }
            }
            let norm = 1.0 / (2 * r + 1) as f32;
            for (i, l) in line.iter_mut().enumerate() {
                *l = acc.map(|v| v * norm);
                let (add, sub) = (get(px, i as i64 + r as i64 + 1), get(px, i as i64 - r as i64));
                for k in 0..4 {
                    acc[k] += add[k] - sub[k];
                }
            }
            for (i, l) in line.iter().enumerate() {
                px[idx(i)] = *l;
            }
        }
    };
    for _ in 0..3 {
        pass(px, (rx / 3.0).round() as usize, true);
        pass(px, (ry / 3.0).round() as usize, false);
    }
}

fn luma(c: [f32; 4]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Apply effects in order. `origin`: the object's origin in pixels (Swirl's
/// centre); `frame` animates Wave's phase.
pub fn apply_effects(c: &mut Canvas, fx: &[&GpEffect], origin: Option<(f32, f32)>, frame: i32) {
    let (w, h) = (c.w, c.h);
    for e in fx {
        match &e.kind {
            EffectKind::Blur { size, .. } => {
                blur(&mut c.px, w, h, size[0], size[1]);
                c.mark_all();
            }
            EffectKind::Colorize { mode, low_color, high_color, factor } => {
                for p in c.px.iter_mut().filter(|p| p[3] > 0.0) {
                    let a = p[3];
                    let rgb = [p[0] / a, p[1] / a, p[2] / a];
                    let l = luma([rgb[0], rgb[1], rgb[2], 1.0]);
                    let (out, na) = match mode {
                        ColorizeMode::GrayScale => ([l; 3], a),
                        ColorizeMode::Sepia => ([(l * 1.07).min(1.0), l * 0.74, l * 0.43], a),
                        ColorizeMode::Duotone => {
                            let c = if l > 0.5 { high_color } else { low_color };
                            ([c[0], c[1], c[2]], a)
                        }
                        ColorizeMode::Custom => ([low_color[0] * l, low_color[1] * l, low_color[2] * l], a),
                        ColorizeMode::Transparent => (rgb, a * (1.0 - factor)),
                    };
                    let f = if *mode == ColorizeMode::Transparent { 1.0 } else { factor.clamp(0.0, 1.0) };
                    for k in 0..3 {
                        p[k] = (rgb[k] + (out[k] - rgb[k]) * f) * na;
                    }
                    p[3] = na;
                }
            }
            EffectKind::Flip { horizontal, vertical } => {
                let old = c.px.clone();
                for y in 0..h {
                    for x in 0..w {
                        let sx = if *horizontal { w - 1 - x } else { x };
                        let sy = if *vertical { h - 1 - y } else { y };
                        c.px[y * w + x] = old[sy * w + sx];
                    }
                }
                c.mark_all();
            }
            EffectKind::Glow { mode, threshold, select_color, glow_color, size, opacity, use_glow_under } => {
                let mut g: Vec<[f32; 4]> = c
                    .px
                    .iter()
                    .map(|&p| {
                        let a = p[3];
                        if a <= 0.0 {
                            return [0.0; 4];
                        }
                        let rgb = [p[0] / a, p[1] / a, p[2] / a];
                        let hit = match mode {
                            GlowMode::Luminance => luma([rgb[0], rgb[1], rgb[2], 1.0]) > *threshold,
                            GlowMode::Color => (0..3).all(|k| (rgb[k] - select_color[k]).abs() <= threshold.max(0.01)),
                        };
                        if hit {
                            [glow_color[0] * a, glow_color[1] * a, glow_color[2] * a, a]
                        } else {
                            [0.0; 4]
                        }
                    })
                    .collect();
                blur(&mut g, w, h, size[0] * 0.5, size[1] * 0.5);
                for (p, q) in c.px.iter_mut().zip(&g) {
                    let q = q.map(|v| v * opacity);
                    if *use_glow_under {
                        let k = 1.0 - p[3];
                        for i in 0..4 {
                            p[i] += q[i] * k;
                        }
                    } else {
                        // Additive glow over.
                        for i in 0..3 {
                            p[i] += q[i];
                        }
                        p[3] = (p[3] + q[3] * (1.0 - p[3])).min(1.0);
                    }
                }
                c.mark_all();
            }
            EffectKind::Pixelate { size } => {
                let (bx, by) = (size[0].max(1) as usize, size[1].max(1) as usize);
                for y0 in (0..h).step_by(by) {
                    for x0 in (0..w).step_by(bx) {
                        let mut acc = [0.0f32; 4];
                        let mut n = 0.0;
                        for y in y0..(y0 + by).min(h) {
                            for x in x0..(x0 + bx).min(w) {
                                let v = c.px[y * w + x];
                                for k in 0..4 {
                                    acc[k] += v[k];
                                }
                                n += 1.0;
                            }
                        }
                        let avg = acc.map(|v| v / n);
                        for y in y0..(y0 + by).min(h) {
                            for x in x0..(x0 + bx).min(w) {
                                c.px[y * w + x] = avg;
                            }
                        }
                    }
                }
                c.mark_all();
            }
            EffectKind::Rim { offset, rim_color, blur: b, .. } => {
                // The rim is where the shape shows but its offset copy does not.
                let mut mask: Vec<[f32; 4]> = (0..w * h)
                    .map(|i| {
                        let (x, y) = ((i % w) as i32 - offset[0], (i / w) as i32 + offset[1]);
                        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 { [0.0; 4] } else { [0.0, 0.0, 0.0, c.px[y as usize * w + x as usize][3]] }
                    })
                    .collect();
                if b[0] > 0.0 || b[1] > 0.0 {
                    blur(&mut mask, w, h, b[0], b[1]);
                }
                for (p, m) in c.px.iter_mut().zip(&mask) {
                    let a = p[3];
                    if a <= 0.0 {
                        continue;
                    }
                    let rim = (a - m[3]).max(0.0) / a;
                    for k in 0..3 {
                        let base = p[k] / a;
                        let masked = base * (1.0 - rim) + rim_color[k] * rim;
                        p[k] = masked * a;
                    }
                }
            }
            EffectKind::Shadow { offset, scale, rotation, color, blur: b } => {
                let (cx, cy) = origin.unwrap_or((w as f32 * 0.5, h as f32 * 0.5));
                let (sr, cr) = (-rotation).sin_cos();
                let mut sh: Vec<[f32; 4]> = (0..w * h)
                    .map(|i| {
                        // Inverse map: where the shadow pixel comes from.
                        let (x, y) = ((i % w) as f32 + 0.5 - offset[0] as f32 - cx, (i / w) as f32 + 0.5 + offset[1] as f32 - cy);
                        let (rx, ry) = (x * cr - y * sr, x * sr + y * cr);
                        let (sx, sy) = (rx / scale[0].max(1e-3) + cx, ry / scale[1].max(1e-3) + cy);
                        let a = sample(&c.px, w, h, sx, sy)[3] * color[3];
                        [color[0] * a, color[1] * a, color[2] * a, a]
                    })
                    .collect();
                if b[0] > 0.0 || b[1] > 0.0 {
                    blur(&mut sh, w, h, b[0], b[1]);
                }
                for (p, s) in c.px.iter_mut().zip(&sh) {
                    let k = 1.0 - p[3];
                    for i in 0..4 {
                        p[i] += s[i] * k;
                    }
                }
                c.mark_all();
            }
            EffectKind::Swirl { radius, angle } => {
                let (cx, cy) = origin.unwrap_or((w as f32 * 0.5, h as f32 * 0.5));
                let old = c.px.clone();
                for y in 0..h {
                    for x in 0..w {
                        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                        let d = (dx * dx + dy * dy).sqrt();
                        if d >= *radius {
                            continue;
                        }
                        let t = 1.0 - d / radius;
                        let a = -angle * t * t;
                        let (s, co) = a.sin_cos();
                        c.px[y * w + x] = sample(&old, w, h, cx + dx * co - dy * s, cy + dx * s + dy * co);
                    }
                }
                c.mark_all();
            }
            EffectKind::Wave { orientation_vertical, amplitude, period, phase } => {
                let old = c.px.clone();
                let ph = phase + frame as f32 * 0.1;
                for y in 0..h {
                    for x in 0..w {
                        let (sx, sy) = if *orientation_vertical {
                            (x as f32 + 0.5 + (y as f32 / period.max(0.1) + ph).sin() * amplitude, y as f32 + 0.5)
                        } else {
                            (x as f32 + 0.5, y as f32 + 0.5 + (x as f32 / period.max(0.1) + ph).sin() * amplitude)
                        };
                        c.px[y * w + x] = sample(&old, w, h, sx, sy);
                    }
                }
                c.mark_all();
            }
        }
    }
}
