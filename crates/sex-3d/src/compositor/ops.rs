// SPDX-License-Identifier: GPL-3.0-only
//! Node dispatch and the per-pixel nodes (colour, keying, masks, converter
//! and utility nodes), after Blender 4.2's compositor shaders
//! (`compositor_*` GLSL and `node_composite_*.cc`).
use super::{filter, glare, gen, pass_buf, pixelwise, transform, Buf, Context, Px, Val};
use crate::nodes::{Node, Prop};
use crate::render::shader::color::{blend, hsl_to_rgb, hsv_to_rgb, rgb_to_hsl, rgb_to_hsv, Blend};
use crate::render::shader::ops::{math, MathOp};
use crate::render::Pass;

pub(super) fn pf(n: &Node, k: &str, d: f32) -> f32 {
    match n.prop(k) {
        Some(Prop::Float(f)) => *f,
        Some(Prop::Int(i)) => *i as f32,
        _ => d,
    }
}
pub(super) fn pi(n: &Node, k: &str, d: i32) -> i32 {
    match n.prop(k) {
        Some(Prop::Int(i)) => *i,
        Some(Prop::Float(f)) => *f as i32,
        _ => d,
    }
}
pub(super) fn pb(n: &Node, k: &str) -> bool {
    n.bool_prop(k)
}
pub(super) fn pe<'a>(n: &'a Node, k: &str) -> &'a str {
    n.enum_prop(k)
}
pub(super) fn pc(n: &Node, k: &str, d: [f32; 4]) -> [f32; 4] {
    match n.prop(k) {
        Some(Prop::Color(c)) => *c,
        Some(Prop::Vector(v)) => [v[0], v[1], v[2], 1.0],
        Some(Prop::Float(f)) => [*f, *f, *f, 1.0],
        _ => d,
    }
}
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn rgb(p: Px) -> [f32; 3] {
    [p[0], p[1], p[2]]
}
fn with_a(c: [f32; 3], a: f32) -> Px {
    [c[0], c[1], c[2], a]
}
fn scale(p: Px, s: f32) -> Px {
    p.map(|v| v * s)
}
pub(super) fn luminance(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}
pub(super) fn srgb_to_linear(v: f32) -> f32 {
    if v < 0.04045 {
        if v < 0.0 {
            0.0
        } else {
            v / 12.92
        }
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}
pub(super) fn linear_to_srgb(v: f32) -> f32 {
    if v < 0.0031308 {
        if v < 0.0 {
            0.0
        } else {
            v * 12.92
        }
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}
fn f1(v: f32) -> Px {
    [v, v, v, 1.0]
}

/// Evaluate one node; returns one value per output socket.
pub(super) fn eval_node(ctx: &Context, n: &Node, ins: &[Val]) -> Vec<Val> {
    let t = ctx.threads;
    let id = n.idname.as_str();
    match id {
        // -------------------------------------------------------- Input
        "CompositorNodeRLayers" => render_layers(ctx, n),
        "CompositorNodeImage" => {
            let img = match n.prop("image") {
                Some(Prop::Id(Some(i))) => ctx.scene.images.get(*i),
                _ => None,
            };
            match img {
                Some(im) if im.width > 0 && im.height > 0 => {
                    let mut b = Buf::from_image(im);
                    if !pb(n, "use_straight_alpha_output") {
                        // Images are stored straight; the compositor works premultiplied.
                        for p in &mut b.px {
                            let a = p[3];
                            p[0] *= a;
                            p[1] *= a;
                            p[2] *= a;
                        }
                    }
                    let alpha = Buf { w: b.w, h: b.h, px: b.px.iter().map(|p| f1(p[3])).collect() };
                    vec![Val::img(b), Val::img(alpha)]
                }
                _ => vec![Val::Single([0.0; 4]), Val::float(0.0)],
            }
        }
        "CompositorNodeRGB" => vec![Val::Single(n.outputs[0].value.as_color())],
        "CompositorNodeValue" => vec![Val::float(n.outputs[0].value.as_float())],
        "CompositorNodeTime" => {
            let (s, e) = (pi(n, "frame_start", 1) as f32, pi(n, "frame_end", 250) as f32);
            let x = if e > s { ((ctx.frame as f32 - s) / (e - s)).clamp(0.0, 1.0) } else { 0.0 };
            let y = match n.prop("curve") {
                Some(Prop::Curve(c)) => c.eval(0, x),
                _ => x,
            };
            vec![Val::float(y)]
        }
        "CompositorNodeSceneTime" => vec![Val::float(ctx.frame as f32 / ctx.fps.max(1e-6)), Val::float(ctx.frame as f32)],
        "CompositorNodeBokehImage" => vec![Val::img(filter::bokeh_image(n, 512))],
        "CompositorNodeMask" | "CompositorNodeMovieClip" | "CompositorNodeTexture" | "CompositorNodeKeyingScreen" | "CompositorNodeTrackPos" => {
            n.outputs.iter().map(|_| Val::Single([0.0; 4])).collect()
        }
        // -------------------------------------------------------- Color
        "CompositorNodeAlphaOver" => {
            let premul = pf(n, "premul", 0.0);
            let key = pb(n, "use_premultiply");
            pixelwise(t, ins, 1, move |a, o| o[0] = alpha_over(a[0][0], a[1], a[2], premul, key))
        }
        "CompositorNodeBrightContrast" => {
            let premul = pb(n, "use_premultiply");
            pixelwise(t, ins, 1, move |a, o| o[0] = bright_contrast(a[0], a[1][0], a[2][0], premul))
        }
        "CompositorNodeColorBalance" => {
            let asc = pe(n, "correction_method") == "OFFSET_POWER_SLOPE";
            let (lift, gamma, gain) = (pc(n, "lift", [1.0; 4]), pc(n, "gamma", [1.0; 4]), pc(n, "gain", [1.0; 4]));
            let (offset, power, slope) = (pc(n, "offset", [0.0; 4]), pc(n, "power", [1.0; 4]), pc(n, "slope", [1.0; 4]));
            let basis = pf(n, "offset_basis", 0.0);
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[1];
                let fac = a[0][0].min(1.0);
                let mut r = [0.0; 3];
                for k in 0..3 {
                    r[k] = if asc {
                        (c[k] * slope[k] + offset[k] + basis).max(0.0).powf(power[k])
                    } else {
                        let s = linear_to_srgb(c[k]);
                        let lifted = (s - 1.0) * (2.0 - lift[k]) + 1.0;
                        let gained = (lifted * gain[k]).max(0.0);
                        srgb_to_linear(gained).powf(1.0 / gamma[k].max(1e-6))
                    };
                }
                o[0] = with_a(mix3(rgb(c), r, fac), c[3]);
            })
        }
        "CompositorNodeColorCorrection" => {
            let cc = ColorCorrection::from_node(n);
            pixelwise(t, ins, 1, move |a, o| o[0] = cc.apply(a[0], a[1][0]))
        }
        "CompositorNodeExposure" => pixelwise(t, ins, 1, |a, o| {
            let m = 2f32.powf(a[1][0]);
            o[0] = [a[0][0] * m, a[0][1] * m, a[0][2] * m, a[0][3]];
        }),
        "CompositorNodeGamma" => pixelwise(t, ins, 1, |a, o| {
            let g = a[1][0];
            let c = a[0];
            let p = |v: f32| if v > 0.0 { v.powf(g) } else { v };
            o[0] = [p(c[0]), p(c[1]), p(c[2]), c[3]];
        }),
        "CompositorNodeHueCorrect" => {
            let m = match n.prop("mapping") {
                Some(Prop::Curve(c)) => c.clone(),
                _ => Default::default(),
            };
            let polys: Vec<Vec<[f32; 2]>> = (0..3).map(|i| m.polyline(i).unwrap_or_else(|| vec![[0.0, 0.5], [1.0, 0.5]])).collect();
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[1];
                let mut hsv = rgb_to_hsv(rgb(c));
                let h = hsv[0];
                hsv[0] += crate::nodes::polyline_eval(&polys[0], h) - 0.5;
                hsv[1] *= crate::nodes::polyline_eval(&polys[1], h) * 2.0;
                hsv[2] *= crate::nodes::polyline_eval(&polys[2], h) * 2.0;
                hsv[0] = hsv[0].rem_euclid(1.0);
                hsv[1] = hsv[1].clamp(0.0, 1.0);
                let r = hsv_to_rgb(hsv).map(|v| v.max(0.0));
                o[0] = with_a(mix3(rgb(c), r, a[0][0]), c[3]);
            })
        }
        "CompositorNodeHueSat" => pixelwise(t, ins, 1, |a, o| {
            let c = a[0];
            let mut hsv = rgb_to_hsv(rgb(c));
            hsv[0] = (hsv[0] + a[1][0] + 0.5).rem_euclid(1.0);
            hsv[1] *= a[2][0];
            hsv[2] *= a[3][0];
            let r = hsv_to_rgb(hsv).map(|v| v.max(0.0));
            o[0] = with_a(mix3(rgb(c), r, a[4][0]), c[3]);
        }),
        "CompositorNodeInvert" => {
            let (irgb, ia) = (pb(n, "invert_rgb"), pb(n, "invert_alpha"));
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[1];
                let mut r = c;
                if irgb {
                    for k in 0..3 {
                        r[k] = 1.0 - c[k];
                    }
                }
                if ia {
                    r[3] = 1.0 - c[3];
                }
                let f = a[0][0];
                o[0] = [c[0] + (r[0] - c[0]) * f, c[1] + (r[1] - c[1]) * f, c[2] + (r[2] - c[2]) * f, c[3] + (r[3] - c[3]) * f];
            })
        }
        "CompositorNodeMixRGB" => {
            let mode = Blend::from_name(pe(n, "blend_type"));
            let (use_alpha, clamp) = (pb(n, "use_alpha"), pb(n, "use_clamp"));
            pixelwise(t, ins, 1, move |a, o| o[0] = mix_rgb(mode, a[0][0], a[1], a[2], use_alpha, clamp))
        }
        "CompositorNodeCurveRGB" => {
            let m = match n.prop("mapping") {
                Some(Prop::Curve(c)) => c.clone(),
                _ => Default::default(),
            };
            let polys: Vec<Vec<[f32; 2]>> = (0..4).map(|i| m.polyline(i).unwrap_or_default()).collect();
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[1];
                let (black, white) = (a[2], a[3]);
                let mut r = [0.0; 3];
                for k in 0..3 {
                    let range = white[k] - black[k];
                    let v = if range != 0.0 { (c[k] - black[k]) / range } else { c[k] - black[k] };
                    let v = crate::nodes::polyline_eval(&polys[0], v);
                    r[k] = crate::nodes::polyline_eval(&polys[k + 1], v);
                }
                o[0] = with_a(mix3(rgb(c), r, a[0][0]), c[3]);
            })
        }
        "CompositorNodeTonemap" => vec![filter::tonemap(n, &ins[0], t)],
        "CompositorNodeZcombine" => {
            let use_alpha = pb(n, "use_alpha");
            pixelwise(t, ins, 2, move |a, o| {
                let (c1, z1, c2, z2) = (a[0], a[1][0], a[2], a[3][0]);
                let (front, back) = if z1 < z2 { (c1, c2) } else { (c2, c1) };
                o[0] = if use_alpha {
                    let f = front[3];
                    [back[0] * (1.0 - f) + front[0] * f, back[1] * (1.0 - f) + front[1] * f, back[2] * (1.0 - f) + front[2] * f, back[3].max(front[3])]
                } else {
                    front
                };
                o[1] = f1(z1.min(z2));
            })
        }
        "CompositorNodeConvertColorSpace" => {
            let (from, to) = (pe(n, "from_color_space").to_string(), pe(n, "to_color_space").to_string());
            let to_lin = from == "sRGB" && to != "sRGB";
            let to_srgb = from != "sRGB" && to == "sRGB";
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[0];
                o[0] = if to_lin {
                    [srgb_to_linear(c[0]), srgb_to_linear(c[1]), srgb_to_linear(c[2]), c[3]]
                } else if to_srgb {
                    [linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2]), c[3]]
                } else {
                    c
                };
            })
        }
        "CompositorNodePosterize" => pixelwise(t, ins, 1, |a, o| {
            let s = a[1][0].clamp(2.0, 1024.0);
            let c = a[0];
            o[0] = [(c[0] * s).floor() / s, (c[1] * s).floor() / s, (c[2] * s).floor() / s, c[3]];
        }),
        "CompositorNodePremulKey" => {
            let to_premul = pe(n, "mapping") != "PREMUL_TO_STRAIGHT";
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[0];
                o[0] = if to_premul {
                    [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]]
                } else if c[3] != 0.0 {
                    [c[0] / c[3], c[1] / c[3], c[2] / c[3], c[3]]
                } else {
                    c
                };
            })
        }
        "CompositorNodeSetAlpha" => {
            let replace = pe(n, "mode") == "REPLACE_ALPHA";
            pixelwise(t, ins, 1, move |a, o| {
                let (c, al) = (a[0], a[1][0]);
                o[0] = if replace { [c[0], c[1], c[2], al] } else { scale(c, al) };
            })
        }
        // -------------------------------------------------------- Keying
        "CompositorNodeLumaMatte" => {
            let (hi, lo) = (pf(n, "limit_max", 1.0), pf(n, "limit_min", 0.0));
            pixelwise(t, ins, 2, move |a, o| {
                let c = a[0];
                let l = luminance(rgb(c));
                let alpha = ((l - lo) / (hi - lo)).clamp(0.0, 1.0);
                matte_out(o, c, alpha);
            })
        }
        "CompositorNodeDiffMatte" => {
            let (tol, fall) = (pf(n, "tolerance", 0.1), pf(n, "falloff", 0.1));
            pixelwise(t, ins, 2, move |a, o| {
                let (c, k) = (a[0], a[1]);
                let d = ((c[0] - k[0]).abs() + (c[1] - k[1]).abs() + (c[2] - k[2]).abs()) / 3.0;
                let alpha = if d > tol + fall { c[3] } else { (d - tol).max(0.0) / fall.max(1e-6) };
                matte_out(o, c, alpha);
            })
        }
        "CompositorNodeDistanceMatte" => {
            let (tol, fall) = (pf(n, "tolerance", 0.1), pf(n, "falloff", 0.1));
            let ycc = pe(n, "channel") == "YCC";
            pixelwise(t, ins, 2, move |a, o| {
                let (c, k) = (a[0], a[1]);
                let d = if ycc {
                    let (cy, ky) = (rgb_to_ycc(rgb(c), YccMode::Bt709), rgb_to_ycc(rgb(k), YccMode::Bt709));
                    ((cy[1] - ky[1]).powi(2) + (cy[2] - ky[2]).powi(2)).sqrt()
                } else {
                    ((c[0] - k[0]).powi(2) + (c[1] - k[1]).powi(2) + (c[2] - k[2]).powi(2)).sqrt()
                };
                let alpha = if d > tol + fall { c[3] } else { (d - tol).max(0.0) / fall.max(1e-6) };
                matte_out(o, c, alpha);
            })
        }
        "CompositorNodeColorMatte" => {
            let (he, se, ve) = (pf(n, "color_hue", 0.01), pf(n, "color_saturation", 0.1), pf(n, "color_value", 0.1));
            pixelwise(t, ins, 2, move |a, o| {
                let (c, k) = (a[0], a[1]);
                let (ch, kh) = (rgb_to_hsv(rgb(c)), rgb_to_hsv(rgb(k)));
                let hd = (ch[0] - kh[0]).abs();
                let inside = hd.min(1.0 - hd) < he && (ch[1] - kh[1]).abs() < se && (ch[2] - kh[2]).abs() < ve;
                let alpha = if inside { 0.0 } else { c[3] };
                matte_out(o, c, alpha);
            })
        }
        "CompositorNodeChromaMatte" => {
            let (acc, cut, fall) = (pf(n, "tolerance", 0.5236), pf(n, "threshold", 0.1745), pf(n, "gain", 1.0));
            pixelwise(t, ins, 2, move |a, o| {
                let (c, k) = (a[0], a[1]);
                let cy = rgb_to_ycc(rgb(c), YccMode::Bt709);
                let ky = rgb_to_ycc(rgb(k), YccMode::Bt709);
                let (cc, kc) = ([cy[1] * 2.0 - 1.0, cy[2] * 2.0 - 1.0], [ky[1] * 2.0 - 1.0, ky[2] * 2.0 - 1.0]);
                let ang = kc[1].atan2(kc[0]);
                let (s, co) = ang.sin_cos();
                let r = [co * cc[0] + s * cc[1], -s * cc[0] + co * cc[1]];
                let fg = r[0] - r[1].abs() / (acc / 2.0).tan();
                let mut alpha = c[3];
                if fg > 0.0 {
                    alpha = 1.0 - fg / fall.max(1e-6);
                    if r[1].atan2(r[0]).abs() < cut / 2.0 {
                        alpha = 0.0;
                    }
                }
                matte_out(o, c, alpha);
            })
        }
        "CompositorNodeChannelMatte" => {
            let space = pe(n, "color_space").to_string();
            let mch = channel_index(pe(n, "matte_channel"));
            let lch = channel_index(pe(n, "limit_channel"));
            let single = pe(n, "limit_method") == "SINGLE";
            let (hi, lo) = (pf(n, "limit_max", 1.0), pf(n, "limit_min", 0.0));
            pixelwise(t, ins, 2, move |a, o| {
                let c = a[0];
                let v = match space.as_str() {
                    "HSV" => rgb_to_hsv(rgb(c)),
                    "YUV" => rgb_to_yuv(rgb(c)),
                    "YCC" => rgb_to_ycc(rgb(c), YccMode::Bt709),
                    _ => rgb(c),
                };
                let mv = v[mch];
                let lv = if single { v[lch] } else { v[(mch + 1) % 3].max(v[(mch + 2) % 3]) };
                let mut alpha = 1.0 - (mv - lv);
                alpha = if alpha > hi {
                    c[3]
                } else if alpha < lo {
                    0.0
                } else {
                    (alpha - lo) / (hi - lo)
                };
                matte_out(o, c, alpha);
            })
        }
        "CompositorNodeColorSpill" => {
            let ch = channel_index(pe(n, "channel"));
            let lch = channel_index(pe(n, "limit_channel"));
            let single = pe(n, "limit_method") == "SIMPLE";
            let ratio = pf(n, "ratio", 1.0);
            let mut sc = [0.0; 3];
            if pb(n, "use_unspill") {
                sc = [pf(n, "unspill_red", 0.0), pf(n, "unspill_green", 0.0), pf(n, "unspill_blue", 0.0)];
                sc[ch] = -sc[ch];
            } else {
                sc[ch] = -1.0;
            }
            pixelwise(t, ins, 1, move |a, o| {
                let c = a[0];
                let limit = if single { c[lch] } else { (c[(ch + 1) % 3] + c[(ch + 2) % 3]) / 2.0 };
                let map = a[1][0] * c[ch] - ratio * limit;
                o[0] = if map > 0.0 { [c[0] + sc[0] * map, c[1] + sc[1] * map, c[2] + sc[2] * map, c[3]] } else { c };
            })
        }
        "CompositorNodeKeying" => filter::keying(n, ins, t),
        "CompositorNodeCryptomatteV2" | "CompositorNodeCryptomatte" => {
            vec![ins.first().cloned().unwrap_or(Val::Single([0.0; 4])), Val::float(0.0), Val::Single([0.0; 4])]
        }
        // -------------------------------------------------------- Mask
        "CompositorNodeBoxMask" | "CompositorNodeEllipseMask" => vec![shape_mask(ctx, n, ins)],
        "CompositorNodeDoubleEdgeMask" => vec![filter::double_edge_mask(n, ins)],
        "CompositorNodeIDMask" => {
            let idx = pi(n, "index", 0) as f32;
            let r = pixelwise(t, ins, 1, move |a, o| o[0] = f1(if a[0][0].round() == idx { 1.0 } else { 0.0 }));
            if pb(n, "use_antialiasing") {
                vec![filter::antialias(&r[0], 0.1, t)]
            } else {
                r
            }
        }
        // -------------------------------------------------------- Utilities
        "CompositorNodeMath" => {
            let op = MathOp::from_name(pe(n, "operation"));
            let clamp = pb(n, "use_clamp");
            pixelwise(t, ins, 1, move |a, o| {
                let mut v = math(op, a[0][0], a[1][0], a[2][0]);
                if clamp {
                    v = v.clamp(0.0, 1.0);
                }
                o[0] = f1(v);
            })
        }
        "CompositorNodeMapRange" => {
            let clamp = pb(n, "use_clamp");
            pixelwise(t, ins, 1, move |a, o| {
                let (v, fmin, fmax, tmin, tmax) = (a[0][0], a[1][0], a[2][0], a[3][0], a[4][0]);
                let mut r = if (fmax - fmin).abs() < 1e-6 { 0.0 } else { tmin + (v - fmin) / (fmax - fmin) * (tmax - tmin) };
                if clamp {
                    r = if tmax > tmin { r.clamp(tmin, tmax) } else { r.clamp(tmax, tmin) };
                }
                o[0] = f1(r);
            })
        }
        "CompositorNodeMapValue" => {
            let (off, size) = (pf(n, "offset", 0.0), pf(n, "size", 1.0));
            let (umin, umax, mn, mx) = (pb(n, "use_min"), pb(n, "use_max"), pf(n, "min", 0.0), pf(n, "max", 1.0));
            pixelwise(t, ins, 1, move |a, o| {
                let mut v = (a[0][0] + off) * size;
                if umin {
                    v = v.max(mn);
                }
                if umax {
                    v = v.min(mx);
                }
                o[0] = f1(v);
            })
        }
        "CompositorNodeNormalize" => vec![filter::normalize(&ins[0])],
        "CompositorNodeLevels" => filter::levels(n, &ins[0]),
        "CompositorNodeSplit" => {
            let fac = pi(n, "factor", 50) as f32 / 100.0;
            let x_axis = pe(n, "axis") != "Y";
            match super::domain(ins, &[]) {
                None => vec![ins[0].clone()],
                Some((w, h)) => {
                    let (a, b) = (ins[0].realize(w, h), ins[1].realize(w, h));
                    vec![Val::img(gen(w, h, t, |x, y| {
                        let first = if x_axis { (x as f32 + 0.5) / w as f32 <= fac } else { ((h - 1 - y) as f32 + 0.5) / h as f32 <= fac };
                        if first {
                            a.px[y * w + x]
                        } else {
                            b.px[y * w + x]
                        }
                    }))]
                }
            }
        }
        "CompositorNodeSwitch" => vec![if pb(n, "check") { ins[1].clone() } else { ins[0].clone() }],
        "CompositorNodeSeparateColor" => {
            let (mode, ycc) = (pe(n, "mode").to_string(), YccMode::from_name(pe(n, "ycc_mode")));
            pixelwise(t, ins, 4, move |a, o| {
                let c = a[0];
                let v = to_mode(&mode, ycc, rgb(c));
                o[0] = f1(v[0]);
                o[1] = f1(v[1]);
                o[2] = f1(v[2]);
                o[3] = f1(c[3]);
            })
        }
        "CompositorNodeCombineColor" => {
            let (mode, ycc) = (pe(n, "mode").to_string(), YccMode::from_name(pe(n, "ycc_mode")));
            pixelwise(t, ins, 1, move |a, o| {
                let c = from_mode(&mode, ycc, [a[0][0], a[1][0], a[2][0]]);
                o[0] = with_a(c, a[3][0]);
            })
        }
        "CompositorNodeSeparateXYZ" => pixelwise(t, ins, 3, |a, o| {
            for k in 0..3 {
                o[k] = f1(a[0][k]);
            }
        }),
        "CompositorNodeCombineXYZ" => pixelwise(t, ins, 1, |a, o| o[0] = [a[0][0], a[1][0], a[2][0], 0.0]),
        "CompositorNodeNormal" => {
            let nv = n.outputs[0].value.as_vector();
            let l = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt().max(1e-12);
            let nn = [nv[0] / l, nv[1] / l, nv[2] / l];
            let mut r = pixelwise(t, ins, 1, move |a, o| o[0] = f1(-(a[0][0] * nn[0] + a[0][1] * nn[1] + a[0][2] * nn[2])));
            vec![Val::Single([nn[0], nn[1], nn[2], 0.0]), r.remove(0)]
        }
        "CompositorNodeCurveVec" => {
            let m = match n.prop("mapping") {
                Some(Prop::Curve(c)) => c.clone(),
                _ => Default::default(),
            };
            let polys: Vec<Vec<[f32; 2]>> = (0..3).map(|i| m.polyline(i).unwrap_or_default()).collect();
            pixelwise(t, ins, 1, move |a, o| {
                let v = a[0];
                o[0] = [crate::nodes::polyline_eval(&polys[0], v[0]), crate::nodes::polyline_eval(&polys[1], v[1]), crate::nodes::polyline_eval(&polys[2], v[2]), 0.0];
            })
        }
        "CompositorNodeValToRGB" => {
            let ramp = match n.prop("color_ramp") {
                Some(Prop::Ramp(r)) => r.clone(),
                _ => Default::default(),
            };
            pixelwise(t, ins, 2, move |a, o| {
                let c = ramp.eval(a[0][0]);
                o[0] = c;
                o[1] = f1(c[3]);
            })
        }
        "CompositorNodeRGBToBW" => pixelwise(t, ins, 1, |a, o| o[0] = f1(luminance(rgb(a[0])))),
        // -------------------------------------------------------- Filter
        "CompositorNodeBlur" => vec![filter::blur(n, ins, t)],
        "CompositorNodeBilateralblur" => vec![filter::bilateral(n, ins, t)],
        "CompositorNodeBokehBlur" => vec![filter::bokeh_blur(n, ins, t)],
        "CompositorNodeDefocus" => vec![filter::defocus(ctx, n, ins, t)],
        "CompositorNodeDenoise" => vec![filter::denoise(n, ins, t)],
        "CompositorNodeDespeckle" => vec![filter::despeckle(n, ins, t)],
        "CompositorNodeDilateErode" => vec![filter::dilate_erode(n, &ins[0], t)],
        "CompositorNodeDBlur" => vec![filter::directional_blur(n, &ins[0], t)],
        "CompositorNodeFilter" => vec![filter::filter3x3(pe(n, "filter_type"), &ins[0], &ins[1], t)],
        "CompositorNodeGlare" => vec![glare::glare(n, &ins[0], t)],
        "CompositorNodeInpaint" => vec![filter::inpaint(pi(n, "distance", 0), &ins[0])],
        "CompositorNodePixelate" => vec![filter::pixelate(pi(n, "pixel_size", 1).max(1) as usize, &ins[0])],
        "CompositorNodeSunBeams" => vec![filter::sun_beams(n, &ins[0], t)],
        "CompositorNodeKuwahara" => vec![filter::kuwahara(n, ins, t)],
        "CompositorNodeAntiAliasing" => vec![filter::antialias(&ins[0], pf(n, "threshold", 1.0) * 0.1, t)],
        "CompositorNodeVecBlur" => vec![ins[0].clone()],
        // -------------------------------------------------------- Transform
        "CompositorNodeRotate" => vec![transform::rotate(n, ins, t)],
        "CompositorNodeScale" => vec![transform::scale(ctx, n, ins, t)],
        "CompositorNodeTransform" => vec![transform::transform(n, ins, t)],
        "CompositorNodeTranslate" => vec![transform::translate(n, ins, t)],
        "CompositorNodeFlip" => vec![transform::flip(pe(n, "axis"), &ins[0])],
        "CompositorNodeCrop" => vec![transform::crop(n, &ins[0])],
        "CompositorNodeDisplace" => vec![transform::displace(ins, t)],
        "CompositorNodeMapUV" => vec![transform::map_uv(ins, t)],
        "CompositorNodeLensdist" => vec![transform::lens_distortion(n, ins, t)],
        "CompositorNodeCornerPin" => transform::corner_pin(ins, t),
        "CompositorNodeStabilize" | "CompositorNodePlaneTrackDeform" | "CompositorNodeMovieDistortion" => {
            let mut v = vec![ins.first().cloned().unwrap_or(Val::Single([0.0; 4]))];
            if n.outputs.len() > 1 {
                v.push(Val::float(1.0));
            }
            v
        }
        _ => n.outputs.iter().map(|_| Val::Single([0.0; 4])).collect(),
    }
}

fn matte_out(o: &mut [Px], c: Px, alpha: f32) {
    let m = alpha.min(c[3]);
    o[0] = scale(c, m);
    o[1] = f1(m);
}

fn channel_index(s: &str) -> usize {
    match s {
        "G" => 1,
        "B" => 2,
        _ => 0,
    }
}

/// Render Layers outputs from the context's render result.
fn render_layers(ctx: &Context, n: &Node) -> Vec<Val> {
    let Some(r) = ctx.render else {
        return n.outputs.iter().map(|_| Val::Single([0.0; 4])).collect();
    };
    let color = Buf::from_image(&r.color);
    let alpha = Buf { w: color.w, h: color.h, px: color.px.iter().map(|p| f1(p[3])).collect() };
    let mut out = vec![Val::img(color), Val::img(alpha)];
    for s in n.outputs.iter().skip(2) {
        let v = match crate::nodes::comp_registry::pass_of_output(&s.identifier) {
            Some(p) => match pass_buf(r, p) {
                Some(mut b) => {
                    if p.channels() == 1 {
                        for q in &mut b.px {
                            *q = f1(q[0]);
                        }
                    } else if matches!(p, Pass::Normal | Pass::Position | Pass::Uv | Pass::DenoisingNormal) {
                        for q in &mut b.px {
                            q[3] = 0.0;
                        }
                    }
                    Val::img(b)
                }
                None => Val::Single([0.0; 4]),
            },
            None => Val::Single([0.0; 4]),
        };
        out.push(v);
    }
    out
}

/// Image fed to Composite/Viewer: realized onto `dom` (or its own size),
/// with the Alpha input and `use_alpha` applied.
pub(super) fn output_image(n: &Node, ins: &[Val], dom: Option<(usize, usize)>) -> Buf {
    let (w, h) = dom.or_else(|| ins[0].size()).unwrap_or((1, 1));
    let img = ins[0].realize(w, h);
    let use_alpha = n.prop("use_alpha").map(|p| matches!(p, Prop::Bool(true))).unwrap_or(true);
    let alpha_linked = ins.get(1).is_some_and(|a| a.image().is_some() || a.single().is_some_and(|p| p[0] != 1.0));
    let alpha = if alpha_linked { Some(ins[1].realize(w, h)) } else { None };
    let mut b = (*img).clone();
    for (i, p) in b.px.iter_mut().enumerate() {
        if !use_alpha {
            p[3] = 1.0;
        } else if let Some(a) = &alpha {
            p[3] = a.px[i][0];
        }
    }
    b
}

/// Alpha Over (Blender's three variants: premultiplied, key, mixed).
pub fn alpha_over(fac: f32, color: Px, over: Px, premul: f32, key: bool) -> Px {
    if premul != 0.0 {
        if over[3] <= 0.0 {
            return color;
        }
        if fac == 1.0 && over[3] >= 1.0 {
            return over;
        }
        let add = 1.0 - premul + over[3] * premul;
        let pm = fac * add;
        let m = 1.0 - fac * over[3];
        return [m * color[0] + pm * over[0], m * color[1] + pm * over[1], m * color[2] + pm * over[2], m * color[3] + fac * over[3]];
    }
    if key {
        if over[3] <= 0.0 {
            return color;
        }
        if fac == 1.0 && over[3] >= 1.0 {
            return over;
        }
        let t = fac * over[3];
        return [color[0] + (over[0] - color[0]) * t, color[1] + (over[1] - color[1]) * t, color[2] + (over[2] - color[2]) * t, color[3] + (1.0 - color[3]) * t];
    }
    if over[3] < 0.0 {
        return color;
    }
    if fac == 1.0 && over[3] >= 1.0 {
        return over;
    }
    let m = 1.0 - fac * over[3];
    [m * color[0] + fac * over[0], m * color[1] + fac * over[1], m * color[2] + fac * over[2], m * color[3] + fac * over[3]]
}

pub fn bright_contrast(c: Px, bright: f32, contrast: f32, premul: bool) -> Px {
    let b = bright / 100.0;
    let mut delta = contrast / 200.0;
    let (mul, off) = if contrast > 0.0 {
        let m = 1.0 / (1.0 - delta * 2.0).max(f32::EPSILON);
        (m, m * (b - delta))
    } else {
        delta = -delta;
        let m = (1.0 - delta * 2.0).max(0.0);
        (m, m * b + delta)
    };
    let mut r = c;
    if premul && c[3] != 0.0 {
        for k in 0..3 {
            r[k] = (c[k] / c[3] * mul + off) * c[3];
        }
    } else {
        for k in 0..3 {
            r[k] = c[k] * mul + off;
        }
    }
    r
}

/// The compositor's Mix node: the shader Mix blend modes, with "Use Alpha"
/// scaling the factor by the second colour's alpha; alpha from the first.
pub fn mix_rgb(mode: Blend, fac: f32, a: Px, b: Px, use_alpha: bool, clamp: bool) -> Px {
    let f = if use_alpha { fac * b[3] } else { fac };
    let mut c = blend(mode, f, rgb(a), rgb(b));
    if clamp {
        c = c.map(|v| v.clamp(0.0, 1.0));
    }
    with_a(c, a[3])
}

struct Range {
    saturation: f32,
    contrast: f32,
    gamma: f32,
    gain: f32,
    lift: f32,
}

struct ColorCorrection {
    channels: [bool; 3],
    start: f32,
    end: f32,
    ranges: [Range; 4],
}

impl ColorCorrection {
    fn from_node(n: &Node) -> ColorCorrection {
        let r = |p: &str| Range {
            saturation: pf(n, &format!("{p}_saturation"), 1.0),
            contrast: pf(n, &format!("{p}_contrast"), 1.0),
            gamma: pf(n, &format!("{p}_gamma"), 1.0),
            gain: pf(n, &format!("{p}_gain"), 1.0),
            lift: pf(n, &format!("{p}_lift"), 0.0),
        };
        ColorCorrection {
            channels: [pb(n, "red"), pb(n, "green"), pb(n, "blue")],
            start: pf(n, "midtones_start", 0.2),
            end: pf(n, "midtones_end", 0.7),
            ranges: [r("master"), r("shadows"), r("midtones"), r("highlights")],
        }
    }

    fn apply(&self, c: Px, mask: f32) -> Px {
        const MARGIN: f32 = 0.10;
        let div = 0.5 / MARGIN;
        let level = (c[0] + c[1] + c[2]) / 3.0;
        let (mut ls, mut lm, mut lh) = (0.0, 0.0, 0.0);
        if level < self.start - MARGIN {
            ls = 1.0;
        } else if level < self.start + MARGIN {
            lm = (level - self.start) * div + 0.5;
            ls = 1.0 - lm;
        } else if level < self.end - MARGIN {
            lm = 1.0;
        } else if level < self.end + MARGIN {
            lh = (level - self.end) * div + 0.5;
            lm = 1.0 - lh;
        } else {
            lh = 1.0;
        }
        let [m, s, mi, h] = &self.ranges;
        let w = |f: fn(&Range) -> f32| ls * f(s) + lm * f(mi) + lh * f(h);
        let contrast = w(|r| r.contrast) * m.contrast;
        let saturation = w(|r| r.saturation) * m.saturation;
        let gamma = w(|r| r.gamma) * m.gamma;
        let gain = w(|r| r.gain) * m.gain;
        let lift = w(|r| r.lift) + m.lift;
        let inv_gamma = 1.0 / gamma;
        let luma = luminance(rgb(c));
        let mut out = c;
        for k in 0..3 {
            let mut v = luma + saturation * (c[k] - luma);
            v = 0.5 + (v - 0.5) * contrast;
            let base = v * gain + lift;
            v = if base >= 0.0 { base.powf(inv_gamma) } else { v };
            v = c[k] + (v - c[k]) * mask.min(1.0);
            if self.channels[k] {
                out[k] = v;
            }
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum YccMode {
    Bt601,
    Bt709,
    Jfif,
}

impl YccMode {
    fn from_name(s: &str) -> YccMode {
        match s {
            "ITUBT601" => YccMode::Bt601,
            "JFIF" => YccMode::Jfif,
            _ => YccMode::Bt709,
        }
    }
    fn matrix(self) -> ([[f32; 3]; 3], [f32; 3]) {
        match self {
            YccMode::Bt601 => ([[0.257, 0.504, 0.098], [-0.148, -0.291, 0.439], [0.439, -0.368, -0.071]], [16.0 / 255.0, 0.5, 0.5]),
            YccMode::Bt709 => ([[0.183, 0.614, 0.062], [-0.101, -0.338, 0.439], [0.439, -0.399, -0.040]], [16.0 / 255.0, 0.5, 0.5]),
            YccMode::Jfif => ([[0.299, 0.587, 0.114], [-0.16874, -0.33126, 0.5], [0.5, -0.41869, -0.08131]], [0.0, 0.5, 0.5]),
        }
    }
}

fn mat_mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2], m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2], m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2]]
}

fn mat_inv(m: &[[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = 1.0 / det;
    [
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d],
    ]
}

/// RGB to Y, Cb, Cr in 0..1 (Blender's `rgb_to_ycc` divided by 255).
pub(super) fn rgb_to_ycc(c: [f32; 3], mode: YccMode) -> [f32; 3] {
    let (m, o) = mode.matrix();
    let v = mat_mul(&m, c);
    [v[0] + o[0], v[1] + o[1], v[2] + o[2]]
}

fn ycc_to_rgb(c: [f32; 3], mode: YccMode) -> [f32; 3] {
    let (m, o) = mode.matrix();
    mat_mul(&mat_inv(&m), [c[0] - o[0], c[1] - o[1], c[2] - o[2]])
}

const YUV: [[f32; 3]; 3] = [[0.2126, 0.7152, 0.0722], [-0.09991, -0.33609, 0.436], [0.615, -0.55861, -0.05639]];

fn rgb_to_yuv(c: [f32; 3]) -> [f32; 3] {
    mat_mul(&YUV, c)
}

fn to_mode(mode: &str, ycc: YccMode, c: [f32; 3]) -> [f32; 3] {
    match mode {
        "HSV" => rgb_to_hsv(c),
        "HSL" => rgb_to_hsl(c),
        "YCC" => rgb_to_ycc(c, ycc),
        "YUV" => rgb_to_yuv(c),
        _ => c,
    }
}

fn from_mode(mode: &str, ycc: YccMode, c: [f32; 3]) -> [f32; 3] {
    match mode {
        "HSV" => hsv_to_rgb(c),
        "HSL" => hsl_to_rgb(c),
        "YCC" => ycc_to_rgb(c, ycc),
        "YUV" => mat_mul(&mat_inv(&YUV), c),
        _ => c,
    }
}

/// Box and Ellipse Mask (Blender 4.2: positions relative to the domain,
/// sizes relative to its width, rotation counter-clockwise).
fn shape_mask(ctx: &Context, n: &Node, ins: &[Val]) -> Val {
    let (w, h) = super::domain(ins, &[]).unwrap_or(ctx.render_size);
    let (w, h) = (w.max(1), h.max(1));
    let ellipse = n.idname == "CompositorNodeEllipseMask";
    let (lx, ly) = (pf(n, "x", 0.5), pf(n, "y", 0.5));
    let half = [pf(n, "mask_width", 0.2) / 2.0, pf(n, "mask_height", 0.1) / 2.0];
    let rot = pf(n, "rotation", 0.0);
    let (s, c) = rot.sin_cos();
    let mode = pe(n, "mask_type").to_string();
    let (base, value) = (ins[0].realize(w, h), ins[1].realize(w, h));
    let aspect = h as f32 / w as f32;
    Val::img(gen(w, h, ctx.threads, |x, y| {
        let yu = h - 1 - y;
        let mut u = [x as f32 / (w.max(2) - 1) as f32 - lx, (yu as f32 / (h.max(2) - 1) as f32 - ly) * aspect];
        u = [c * u[0] + s * u[1], -s * u[0] + c * u[1]];
        let inside = if ellipse {
            (u[0] / half[0]).powi(2) + (u[1] / half[1]).powi(2) < 1.0
        } else {
            u[0].abs() < half[0] && u[1].abs() < half[1]
        };
        let (b, v) = (base.px[y * w + x][0], value.px[y * w + x][0]);
        let r = match mode.as_str() {
            "SUBTRACT" => {
                if inside {
                    (b - v).clamp(0.0, 1.0)
                } else {
                    b
                }
            }
            "MULTIPLY" => {
                if inside {
                    b * v
                } else {
                    0.0
                }
            }
            "NOT" => {
                if inside {
                    if b > 0.0 {
                        0.0
                    } else {
                        v
                    }
                } else {
                    b
                }
            }
            _ => {
                if inside {
                    b.max(v)
                } else {
                    b
                }
            }
        };
        f1(r)
    }))
}
