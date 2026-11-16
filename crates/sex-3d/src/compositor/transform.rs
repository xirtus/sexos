// SPDX-License-Identifier: GPL-3.0-only
//! Transform and distort nodes. Geometry is computed in Blender's
//! bottom-left pixel space ("up" coordinates, y up) and mapped to the
//! top-left buffers when sampling. Rotate, Translate, Transform, Flip,
//! Displace, Map UV, Lens Distortion and Corner Pin keep the canvas;
//! Scale and Crop (with "Crop Image Size") change it.
use super::ops::{pb, pe, pf, pi};
use super::{gen, Buf, Context, Interp, Px, Val};
use crate::nodes::Node;

/// Sample `b` at up-space continuous coordinates.
#[inline]
fn sample_up(b: &Buf, x: f32, y: f32, interp: Interp) -> Px {
    b.sample(x, b.h as f32 - y, interp)
}

/// For each output pixel (up-space centre), the up-space source point.
fn warp(b: &Buf, w: usize, h: usize, t: usize, interp: Interp, f: impl Fn(f32, f32) -> Option<(f32, f32)> + Sync) -> Buf {
    gen(w, h, t, |x, y| {
        let (px, py) = (x as f32 + 0.5, (h - y) as f32 - 0.5);
        match f(px, py) {
            Some((sx, sy)) => sample_up(b, sx, sy, interp),
            None => [0.0; 4],
        }
    })
}

pub(super) fn rotate(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let a = ins[1].as_single()[0];
    if a == 0.0 {
        return ins[0].clone();
    }
    let (s, c) = a.sin_cos();
    let (cx, cy) = (b.w as f32 / 2.0, b.h as f32 / 2.0);
    let interp = Interp::from_name(pe(n, "filter_type"));
    Val::img(warp(b, b.w, b.h, t, interp, |x, y| {
        let (dx, dy) = (x - cx, y - cy);
        Some((c * dx + s * dy + cx, -s * dx + c * dy + cy))
    }))
}

pub(super) fn transform(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let (tx, ty, a, sc) = (ins[1].as_single()[0], ins[2].as_single()[0], ins[3].as_single()[0], ins[4].as_single()[0]);
    if tx == 0.0 && ty == 0.0 && a == 0.0 && sc == 1.0 {
        return ins[0].clone();
    }
    let sc = if sc.abs() < 1e-6 { 1e-6 } else { sc };
    let (s, c) = a.sin_cos();
    let (cx, cy) = (b.w as f32 / 2.0, b.h as f32 / 2.0);
    let interp = Interp::from_name(pe(n, "filter_type"));
    Val::img(warp(b, b.w, b.h, t, interp, |x, y| {
        let (dx, dy) = ((x - cx - tx) / sc, (y - cy - ty) / sc);
        Some((c * dx + s * dy + cx, -s * dx + c * dy + cy))
    }))
}

pub(super) fn translate(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let (mut tx, mut ty) = (ins[1].as_single()[0], ins[2].as_single()[0]);
    if pb(n, "use_relative") {
        tx *= b.w as f32;
        ty *= b.h as f32;
    }
    let wrap = pe(n, "wrap_axis").to_string();
    let (wx, wy) = (matches!(wrap.as_str(), "XAXIS" | "BOTH"), matches!(wrap.as_str(), "YAXIS" | "BOTH"));
    let interp = Interp::from_name(pe(n, "interpolation"));
    let (w, h) = (b.w as f32, b.h as f32);
    Val::img(warp(b, b.w, b.h, t, interp, |x, y| {
        let mut sx = x - tx;
        let mut sy = y - ty;
        if wx {
            sx = sx.rem_euclid(w);
        }
        if wy {
            sy = sy.rem_euclid(h);
        }
        Some((sx, sy))
    }))
}

/// Scale: relative factors, absolute pixels, the scene's render percentage
/// or the render size (stretch, fit or crop). Changes the canvas size.
pub(super) fn scale(ctx: &Context, n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let (x, y) = (ins[1].as_single()[0], ins[2].as_single()[0]);
    let (w, h) = (b.w as f32, b.h as f32);
    let resample = |nw: usize, nh: usize| -> Buf {
        let (nw, nh) = (nw.max(1), nh.max(1));
        let (fx, fy) = (w / nw as f32, h / nh as f32);
        gen(nw, nh, t, |i, j| b.bilinear_clamped((i as f32 + 0.5) * fx, (j as f32 + 0.5) * fy))
    };
    match pe(n, "space") {
        "ABSOLUTE" => Val::img(resample(x.round().max(1.0) as usize, y.round().max(1.0) as usize)),
        "SCENE_SIZE" => {
            let s = ctx.render_scale;
            Val::img(resample((w * s).round() as usize, (h * s).round() as usize))
        }
        "RENDER_SIZE" => {
            let (rw, rh) = (ctx.render_size.0.max(1), ctx.render_size.1.max(1));
            let method = pe(n, "frame_method");
            if method == "STRETCH" {
                return Val::img(resample(rw, rh));
            }
            let f = if method == "FIT" { (rw as f32 / w).min(rh as f32 / h) } else { (rw as f32 / w).max(rh as f32 / h) };
            let (sw, sh) = ((w * f).round() as usize, (h * f).round() as usize);
            let scaled = resample(sw, sh);
            let (ox, oy) = (pf(n, "offset_x", 0.0) * rw as f32, pf(n, "offset_y", 0.0) * rh as f32);
            let x0 = (rw as f32 - sw as f32) / 2.0 + ox;
            let y0 = (rh as f32 - sh as f32) / 2.0 - oy;
            Val::img(gen(rw, rh, t, |i, j| scaled.get((i as f32 - x0).floor() as isize, (j as f32 - y0).floor() as isize)))
        }
        _ => Val::img(resample((w * x).round() as usize, (h * y).round() as usize)),
    }
}

pub(super) fn flip(axis: &str, v: &Val) -> Val {
    let Some(b) = v.image() else { return v.clone() };
    let (fx, fy) = (axis != "Y", axis != "X");
    let (w, h) = (b.w, b.h);
    Val::img(gen(w, h, 1, |x, y| b.px[(if fy { h - 1 - y } else { y }) * w + if fx { w - 1 - x } else { x }]))
}

/// Crop to a rectangle (pixels or relative, Blender's y up); with "Crop
/// Image Size" the canvas becomes the rectangle, else outside is cleared.
pub(super) fn crop(n: &Node, v: &Val) -> Val {
    let Some(b) = v.image() else { return v.clone() };
    let (w, h) = (b.w as i64, b.h as i64);
    let (x1, x2, y1, y2) = if pb(n, "relative") {
        let r = |k: &str, s: i64| (pf(n, k, 0.0) * s as f32) as i64;
        (r("rel_min_x", w), r("rel_max_x", w), r("rel_min_y", h), r("rel_max_y", h))
    } else {
        (pi(n, "min_x", 0) as i64, pi(n, "max_x", 0) as i64, pi(n, "min_y", 0) as i64, pi(n, "max_y", 0) as i64)
    };
    let xmin = x1.min(x2).clamp(0, w);
    let xmax = (x1.max(x2) + 1).clamp(0, w);
    let ymin = y1.min(y2).clamp(0, h);
    let ymax = (y1.max(y2) + 1).clamp(0, h);
    if pb(n, "use_crop_size") {
        let (cw, ch) = ((xmax - xmin).max(1) as usize, (ymax - ymin).max(1) as usize);
        // Up-space rows ymin..ymax are top-left rows h-ymax..h-ymin.
        let top = (h - ymax).max(0) as usize;
        Val::img(gen(cw, ch, 1, |x, y| b.get(x as isize + xmin as isize, (y + top) as isize)))
    } else {
        Val::img(gen(b.w, b.h, 1, |x, y| {
            let yu = h - 1 - y as i64;
            if (x as i64) >= xmin && (x as i64) < xmax && yu >= ymin && yu < ymax {
                b.px[y * b.w + x]
            } else {
                [0.0; 4]
            }
        }))
    }
}

pub(super) fn displace(ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (b.w, b.h);
    let (v, xs, ys) = (ins[1].realize(w, h), ins[2].realize(w, h), ins[3].realize(w, h));
    Val::img(gen(w, h, t, |x, y| {
        let i = y * w + x;
        let (dx, dy) = (v.px[i][0] * xs.px[i][0], v.px[i][1] * ys.px[i][0]);
        let (px, py) = (x as f32 + 0.5, (h - y) as f32 - 0.5);
        sample_up(b, px - dx, py - dy, Interp::Bilinear)
    }))
}

pub(super) fn map_uv(ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let Some((w, h)) = ins[1].size() else {
        let uv = ins[1].as_single();
        return Val::Single(sample_up(b, uv[0] * b.w as f32, uv[1] * b.h as f32, Interp::Bilinear));
    };
    let uv = ins[1].realize(w, h);
    Val::img(gen(w, h, t, |x, y| {
        let p = uv.px[y * w + x];
        if !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[1]) {
            return [0.0; 4];
        }
        sample_up(b, p[0] * b.w as f32, p[1] * b.h as f32, Interp::Bilinear)
    }))
}

/// Lens Distortion: Blender's screen-space radial model with per-channel
/// dispersion and "Fit".
pub(super) fn lens_distortion(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(b) = ins[0].image() else { return ins[0].clone() };
    let k = ins[1].as_single()[0].clamp(-0.999, 1.0);
    let disp = ins[2].as_single()[0].clamp(0.0, 1.0) / 4.0;
    if k == 0.0 && disp == 0.0 {
        return ins[0].clone();
    }
    let kr = (k + disp).clamp(-0.999, 1.0);
    let kg = k;
    let kb = (k - disp).clamp(-0.999, 1.0);
    let maxk = kr.max(kg).max(kb);
    let sc = if pb(n, "use_fit") && maxk > 0.0 { 1.0 / (1.0 + 2.0 * maxk) } else { 1.0 / (1.0 + maxk) };
    let (w, h) = (b.w, b.h);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    Val::img(gen(w, h, t, |x, y| {
        let (px, py) = (x as f32 + 0.5, (h - y) as f32 - 0.5);
        let u = sc * (px - cx) / cx;
        let v = sc * (py - cy) / cy;
        let r2 = u * u + v * v;
        let mut out = [0.0; 4];
        let mut alpha = 0.0;
        for (c, kk) in [kr, kg, kb].iter().enumerate() {
            let tt = 1.0 - 4.0 * kk * r2;
            if tt < 0.0 {
                continue;
            }
            let d = 1.0 / (1.0 + tt.sqrt());
            let (sx, sy) = (cx + 2.0 * u * d * cx, cy + 2.0 * v * d * cy);
            let p = sample_up(b, sx, sy, Interp::Bilinear);
            out[c] = p[c];
            alpha += p[3] / 3.0;
        }
        out[3] = alpha;
        out
    }))
}

/// Corner Pin: map the image onto the quad given by the four corners
/// (relative coordinates, y up); outputs Image and Plane.
pub(super) fn corner_pin(ins: &[Val], t: usize) -> Vec<Val> {
    let Some(b) = ins[0].image() else { return vec![ins[0].clone(), Val::float(1.0)] };
    let c = |i: usize| {
        let v = ins[i].as_single();
        [v[0], v[1]]
    };
    // p0..p3 = images of (0,0), (1,0), (1,1), (0,1).
    let (p0, p1, p2, p3) = (c(3), c(4), c(2), c(1));
    let m = square_to_quad(p0, p1, p2, p3);
    let Some(inv) = inverse3(&m) else { return vec![Val::Single([0.0; 4]), Val::float(0.0)] };
    let (w, h) = (b.w, b.h);
    let mut plane = Buf::new(w, h, [0.0, 0.0, 0.0, 1.0]);
    let img = gen(w, h, t, |x, y| {
        let (gx, gy) = ((x as f32 + 0.5) / w as f32, ((h - y) as f32 - 0.5) / h as f32);
        let q = [inv[0][0] * gx + inv[0][1] * gy + inv[0][2], inv[1][0] * gx + inv[1][1] * gy + inv[1][2], inv[2][0] * gx + inv[2][1] * gy + inv[2][2]];
        if q[2].abs() < 1e-12 {
            return [0.0; 4];
        }
        let (u, v) = (q[0] / q[2], q[1] / q[2]);
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
            return [0.0; 4];
        }
        sample_up(b, u * w as f32, v * h as f32, Interp::Bilinear)
    });
    for y in 0..h {
        for x in 0..w {
            let (gx, gy) = ((x as f32 + 0.5) / w as f32, ((h - y) as f32 - 0.5) / h as f32);
            let q = [inv[0][0] * gx + inv[0][1] * gy + inv[0][2], inv[1][0] * gx + inv[1][1] * gy + inv[1][2], inv[2][0] * gx + inv[2][1] * gy + inv[2][2]];
            let inside = q[2].abs() > 1e-12 && (0.0..=1.0).contains(&(q[0] / q[2])) && (0.0..=1.0).contains(&(q[1] / q[2]));
            let v = inside as i32 as f32;
            plane.px[y * w + x] = [v, v, v, 1.0];
        }
    }
    vec![Val::img(img), Val::img(plane)]
}

/// Heckbert's square-to-quad projective map as a 3x3 matrix (rows).
fn square_to_quad(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) -> [[f32; 3]; 3] {
    let (dx1, dx2, dx3) = (p1[0] - p2[0], p3[0] - p2[0], p0[0] - p1[0] + p2[0] - p3[0]);
    let (dy1, dy2, dy3) = (p1[1] - p2[1], p3[1] - p2[1], p0[1] - p1[1] + p2[1] - p3[1]);
    if dx3.abs() < 1e-9 && dy3.abs() < 1e-9 {
        return [[p1[0] - p0[0], p3[0] - p0[0], p0[0]], [p1[1] - p0[1], p3[1] - p0[1], p0[1]], [0.0, 0.0, 1.0]];
    }
    let det = dx1 * dy2 - dx2 * dy1;
    let g = (dx3 * dy2 - dx2 * dy3) / det;
    let hh = (dx1 * dy3 - dx3 * dy1) / det;
    [[p1[0] - p0[0] + g * p1[0], p3[0] - p0[0] + hh * p3[0], p0[0]], [p1[1] - p0[1] + g * p1[1], p3[1] - p0[1] + hh * p3[1], p0[1]], [g, hh, 1.0]]
}

fn inverse3(m: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [(m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d, (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d, (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d],
        [(m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d, (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d, (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d],
        [(m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d, (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d, (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d],
    ])
}
