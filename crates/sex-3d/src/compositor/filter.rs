// SPDX-License-Identifier: GPL-3.0-only
//! Neighbourhood nodes: blurs, Defocus, Denoise, Despeckle, Dilate/Erode,
//! the 3x3 Filter kernels, Inpaint, Pixelate, Sun Beams, Kuwahara,
//! Anti-Aliasing, Keying, Double Edge Mask, Normalize, Levels, Tonemap and
//! the Bokeh Image generator. Images are sampled clamped to their edges
//! unless noted (Blender's `texture_load`).
use super::ops::{luminance, pb, pe, pf, pi};
use super::{gen, par_rows, Buf, Context, Px, Val};
use crate::nodes::Node;
use std::f32::consts::PI;

fn f1(v: f32) -> Px {
    [v, v, v, 1.0]
}

/// Blender's `RE_filter_value` for the blur filter types, `x` in 0..1 of
/// the radius.
pub(super) fn filter_value(kind: &str, x: f32) -> f32 {
    const GAUSSFAC: f32 = 1.6;
    let x = x.abs();
    match kind {
        "FLAT" => {
            if x > 1.0 {
                0.0
            } else {
                1.0
            }
        }
        "TENT" => (1.0 - x).max(0.0),
        "QUAD" => {
            let x = x * GAUSSFAC;
            if x < 0.5 {
                0.75 - x * x
            } else if x < 1.5 {
                0.5 * (x - 1.5) * (x - 1.5)
            } else {
                0.0
            }
        }
        "CUBIC" => {
            let x = x * GAUSSFAC;
            if x < 1.0 {
                0.5 * x * x * x - x * x + 2.0 / 3.0
            } else if x < 2.0 {
                (2.0 - x).powi(3) / 6.0
            } else {
                0.0
            }
        }
        "CATROM" => {
            let x = x * GAUSSFAC;
            if x < 1.0 {
                1.5 * x * x * x - 2.5 * x * x + 1.0
            } else if x < 2.0 {
                -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
            } else {
                0.0
            }
        }
        "MITCH" => {
            let x = x * GAUSSFAC;
            let (b, c) = (1.0 / 3.0, 1.0 / 3.0);
            let (p0, p2, p3) = ((6.0 - 2.0 * b) / 6.0, (-18.0 + 12.0 * b + 6.0 * c) / 6.0, (12.0 - 9.0 * b - 6.0 * c) / 6.0);
            let (q0, q1, q2, q3) = ((8.0 * b + 24.0 * c) / 6.0, (-12.0 * b - 48.0 * c) / 6.0, (6.0 * b + 30.0 * c) / 6.0, (-b - 6.0 * c) / 6.0);
            if x < 1.0 {
                p0 + x * x * (p2 + x * p3)
            } else if x < 2.0 {
                q0 + x * (q1 + x * (q2 + x * q3))
            } else {
                0.0
            }
        }
        _ => {
            if x > 1.0 {
                return 0.0;
            }
            let x = x * 3.0 * GAUSSFAC;
            (-x * x / (2.0 * GAUSSFAC * GAUSSFAC)).exp()
        }
    }
}

/// Normalized symmetric kernel of `radius` pixels (index 0 = offset -r).
fn kernel(kind: &str, radius: f32) -> Vec<f32> {
    let r = radius.ceil() as isize;
    if r <= 0 || radius <= 0.0 {
        return vec![1.0];
    }
    let mut k: Vec<f32> = (-r..=r).map(|i| filter_value(kind, i as f32 / radius)).collect();
    let s: f32 = k.iter().sum();
    if s > 0.0 {
        for v in &mut k {
            *v /= s;
        }
    }
    k
}

/// Separable convolution, edges clamped.
pub(super) fn convolve(b: &Buf, kx: &[f32], ky: &[f32], threads: usize) -> Buf {
    let (w, h) = (b.w, b.h);
    let rx = (kx.len() / 2) as isize;
    let ry = (ky.len() / 2) as isize;
    let mut tmp = Buf::new(w, h, [0.0; 4]);
    par_rows(&mut tmp.px, w, threads, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = [0.0; 4];
            for (i, k) in kx.iter().enumerate() {
                let p = b.clamped(x as isize + i as isize - rx, y as isize);
                for c in 0..4 {
                    s[c] += p[c] * k;
                }
            }
            *o = s;
        }
    });
    let mut out = Buf::new(w, h, [0.0; 4]);
    par_rows(&mut out.px, w, threads, |y, row| {
        for (x, o) in row.iter_mut().enumerate() {
            let mut s = [0.0; 4];
            for (i, k) in ky.iter().enumerate() {
                let p = tmp.clamped(x as isize, y as isize + i as isize - ry);
                for c in 0..4 {
                    s[c] += p[c] * k;
                }
            }
            *o = s;
        }
    });
    out
}

/// Gaussian blur of `radius` pixels (used by Glare, Keying, ...).
pub(super) fn gaussian(b: &Buf, radius: f32, threads: usize) -> Buf {
    let k = kernel("GAUSS", radius);
    convolve(b, &k, &k, threads)
}

fn pad(b: &Buf, px: usize, py: usize) -> Buf {
    let (w, h) = (b.w + 2 * px, b.h + 2 * py);
    let mut o = Buf::new(w, h, [0.0; 4]);
    for y in 0..b.h {
        o.px[(y + py) * w + px..(y + py) * w + px + b.w].copy_from_slice(&b.px[y * b.w..(y + 1) * b.w]);
    }
    o
}

/// Blur: filter types, relative size, aspect correction, extended bounds,
/// gamma correction, bokeh (elliptic) kernel and variable size.
pub(super) fn blur(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (src.w as f32, src.h as f32);
    let (mut sx, mut sy) = if pb(n, "use_relative") {
        let (fx, fy) = (pf(n, "factor_x", 0.0) / 100.0, pf(n, "factor_y", 0.0) / 100.0);
        match pe(n, "aspect_correction") {
            "Y" => (fx * w, fy * w),
            "X" => (fx * h, fy * h),
            _ => (fx * w, fy * h),
        }
    } else {
        (pi(n, "size_x", 0) as f32, pi(n, "size_y", 0) as f32)
    };
    let variable = pb(n, "use_variable_size") && ins[1].image().is_some();
    if !variable {
        let s = ins[1].as_single()[0];
        sx *= s;
        sy *= s;
    }
    if sx <= 0.0 && sy <= 0.0 {
        return ins[0].clone();
    }
    let mut b = (*src).clone();
    if pb(n, "use_extended_bounds") {
        b = pad(&b, sx.ceil() as usize, sy.ceil() as usize);
    }
    let gamma = pb(n, "use_gamma_correction");
    if gamma {
        for p in &mut b.px {
            for c in 0..3 {
                p[c] = p[c].max(0.0).sqrt();
            }
        }
    }
    let kind = pe(n, "filter_type").to_string();
    let mut out = if variable || pb(n, "use_bokeh") {
        let size = if variable { Some(ins[1].realize(b.w, b.h)) } else { None };
        let bokeh = pb(n, "use_bokeh");
        let (rx, ry) = (sx.ceil() as isize, sy.ceil() as isize);
        gen(b.w, b.h, t, |x, y| {
            let f = size.as_ref().map_or(1.0, |s| s.px[y * b.w + x][0].clamp(0.0, 1.0));
            let (ex, ey) = (sx * f, sy * f);
            if ex < 0.5 && ey < 0.5 {
                return b.px[y * b.w + x];
            }
            let mut s = [0.0; 4];
            let mut wsum = 0.0;
            for dy in -ry..=ry {
                for dx in -rx..=rx {
                    let (u, v) = (if ex > 0.0 { dx as f32 / ex } else { dx as f32 * 1e9 }, if ey > 0.0 { dy as f32 / ey } else { dy as f32 * 1e9 });
                    let k = if bokeh {
                        if u * u + v * v <= 1.0 {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        filter_value(&kind, u) * filter_value(&kind, v)
                    };
                    if k <= 0.0 {
                        continue;
                    }
                    let p = b.clamped(x as isize + dx, y as isize + dy);
                    for c in 0..4 {
                        s[c] += p[c] * k;
                    }
                    wsum += k;
                }
            }
            if wsum > 0.0 {
                s.map(|v| v / wsum)
            } else {
                b.px[y * b.w + x]
            }
        })
    } else {
        convolve(&b, &kernel(&kind, sx), &kernel(&kind, sy), t)
    };
    if gamma {
        for p in &mut out.px {
            for c in 0..3 {
                p[c] *= p[c].max(0.0);
            }
        }
    }
    Val::img(out)
}

pub(super) fn bilateral(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (src.w, src.h);
    let det = ins[1].realize(w, h);
    let iters = pi(n, "iterations", 1).max(1) as f32;
    let sc = pf(n, "sigma_color", 0.3).max(1e-4);
    let ss = pf(n, "sigma_space", 5.0).max(1e-4);
    let r = (ss + iters).ceil() as isize;
    Val::img(gen(w, h, t, |x, y| {
        let c0 = det.px[y * w + x];
        let mut s = [0.0; 4];
        let mut ws = 0.0;
        for dy in -r..=r {
            for dx in -r..=r {
                let d = det.clamped(x as isize + dx, y as isize + dy);
                let cd = ((d[0] - c0[0]).powi(2) + (d[1] - c0[1]).powi(2) + (d[2] - c0[2]).powi(2)).sqrt();
                let k = (-((dx * dx + dy * dy) as f32) / (2.0 * ss * ss)).exp() * (-(cd * cd) / (2.0 * sc * sc)).exp();
                let p = src.clamped(x as isize + dx, y as isize + dy);
                for c in 0..4 {
                    s[c] += p[c] * k;
                }
                ws += k;
            }
        }
        s.map(|v| v / ws.max(1e-12))
    }))
}

/// Bokeh Image: a polygon (flaps, angle, rounding) with a catadioptric hole
/// and lens shift dispersing red and blue, size x size pixels.
pub(super) fn bokeh_image(n: &Node, size: usize) -> Buf {
    let flaps = pi(n, "flaps", 5).max(3) as f32;
    let angle = pf(n, "angle", 0.0);
    let rounding = pf(n, "rounding", 0.0).clamp(0.0, 1.0);
    let cata = pf(n, "catadioptric", 0.0).clamp(0.0, 1.0);
    let shift = pf(n, "shift", 0.0).clamp(-1.0, 1.0);
    let ext = 2.0 * PI / flaps;
    let px = 2.0 / size as f32;
    let shape = |x: f32, y: f32, radius: f32| -> f32 {
        let d = (x * x + y * y).sqrt();
        let a = (y.atan2(x) - angle - PI / 2.0).rem_euclid(ext) - ext / 2.0;
        let edge = (ext / 2.0).cos() / a.cos();
        let edge = edge + (1.0 - edge) * rounding;
        let outer = ((radius * edge - d) / px + 0.5).clamp(0.0, 1.0);
        let inner = ((d - radius * cata) / px + 0.5).clamp(0.0, 1.0);
        if cata > 0.0 {
            outer * inner
        } else {
            outer
        }
    };
    gen(size, size, 1, |x, y| {
        let u = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
        let v = 1.0 - (y as f32 + 0.5) / size as f32 * 2.0;
        let r = shape(u, v, 1.0 - shift.max(0.0));
        let g = shape(u, v, 1.0 - shift.abs() / 2.0);
        let b = shape(u, v, 1.0 + shift.min(0.0));
        [r, g, b, 1.0]
    })
}

pub(super) fn bokeh_blur(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (src.w, src.h);
    let max_dim = w.max(h) as f32;
    let bokeh = match ins[1].image() {
        Some(b) => (*b).clone(),
        None => Buf::new(1, 1, ins[1].as_single()),
    };
    let variable = pb(n, "use_variable_size") && ins[2].image().is_some();
    let size_img = if variable { Some(ins[2].realize(w, h)) } else { None };
    let blur_max = pf(n, "blur_max", 16.0);
    let base = ins[2].as_single()[0];
    let radius_of = |x: usize, y: usize| -> f32 {
        match &size_img {
            Some(s) => (s.px[y * w + x][0] * max_dim / 100.0).min(blur_max),
            None => base * max_dim / 100.0,
        }
    };
    let bbox = ins[3].realize(w, h);
    let maxr = if variable { blur_max } else { base * max_dim / 100.0 };
    let rr = maxr.ceil() as isize;
    if rr < 1 {
        return ins[0].clone();
    }
    let src = src.clone();
    Val::img(gen(w, h, t, |x, y| {
        if bbox.px[y * w + x][0] <= 0.0 {
            return src.px[y * w + x];
        }
        let r = radius_of(x, y);
        if r < 0.5 {
            return src.px[y * w + x];
        }
        let ri = r.ceil() as isize;
        let mut s = [0.0; 4];
        let mut ws = [0.0; 4];
        for dy in -ri..=ri {
            for dx in -ri..=ri {
                let u = (dx as f32 / r * 0.5 + 0.5) * bokeh.w as f32;
                let v = (0.5 - dy as f32 / r * 0.5) * bokeh.h as f32;
                if !(0.0..bokeh.w as f32).contains(&u) || !(0.0..bokeh.h as f32).contains(&v) {
                    continue;
                }
                let k = bokeh.bilinear_clamped(u, v);
                let p = src.clamped(x as isize + dx, y as isize + dy);
                for c in 0..4 {
                    let kc = if c == 3 { (k[0] + k[1] + k[2]) / 3.0 } else { k[c] };
                    s[c] += p[c] * kc;
                    ws[c] += kc;
                }
            }
        }
        let mut o = [0.0; 4];
        for c in 0..4 {
            o[c] = if ws[c] > 0.0 { s[c] / ws[c] } else { src.px[y * w + x][c] };
        }
        o
    }))
}

/// Is (u, v) (in units of the radius) inside a regular `sides`-gon rotated
/// by `rot` (a circle for fewer than 3 sides)?
fn in_shape(u: f32, v: f32, sides: u32, rot: f32) -> bool {
    let d = (u * u + v * v).sqrt();
    if sides < 3 {
        return d <= 1.0;
    }
    let ext = 2.0 * PI / sides as f32;
    let a = (v.atan2(u) - rot).rem_euclid(ext) - ext / 2.0;
    d <= (ext / 2.0).cos() / a.cos()
}

/// Defocus: per-pixel circle of confusion from the Z input (a depth with
/// "Use Z-Buffer", else a blur amount), gathered from neighbours whose
/// bokeh covers the pixel.
pub(super) fn defocus(ctx: &Context, n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (src.w, src.h);
    let z = ins[1].realize(w, h);
    let blur_max = pf(n, "blur_max", 16.0).max(0.0);
    let fstop = pf(n, "f_stop", 128.0).max(0.5);
    let zscale = pf(n, "z_scale", 1.0);
    let sides = match pe(n, "bokeh") {
        "OCTAGON" => 8,
        "HEPTAGON" => 7,
        "HEXAGON" => 6,
        "PENTAGON" => 5,
        "SQUARE" => 4,
        "TRIANGLE" => 3,
        _ => 0,
    };
    let rot = pf(n, "angle", 0.0);
    let cam = ctx.camera.unwrap_or(super::CameraInfo { focal_mm: 50.0, sensor_mm: 36.0, focus_m: 10.0 });
    let use_z = pb(n, "use_zbuffer");
    let radius: Vec<f32> = z
        .px
        .iter()
        .map(|p| {
            let r = if use_z {
                let f = cam.focal_mm / 1000.0;
                let s = cam.focus_m.max(f * 1.001);
                let d = p[0].max(1e-4);
                let aperture = f / fstop;
                let coc = (aperture * f * (d - s) / (d * (s - f))).abs();
                coc / (cam.sensor_mm / 1000.0) * w as f32 / 2.0
            } else {
                p[0] * zscale
            };
            r.clamp(0.0, blur_max)
        })
        .collect();
    let rr = blur_max.ceil() as isize;
    let src = src.clone();
    Val::img(gen(w, h, t, |x, y| {
        let mut s = [0.0; 4];
        let mut ws = 0.0;
        for dy in -rr..=rr {
            for dx in -rr..=rr {
                let (qx, qy) = (x as isize + dx, y as isize + dy);
                if qx < 0 || qy < 0 || qx as usize >= w || qy as usize >= h {
                    continue;
                }
                let qi = qy as usize * w + qx as usize;
                let r = radius[qi].max(0.5);
                if !in_shape(dx as f32 / r, -dy as f32 / r, sides, rot) {
                    continue;
                }
                let k = 1.0 / (r * r);
                let p = src.px[qi];
                for c in 0..4 {
                    s[c] += p[c] * k;
                }
                ws += k;
            }
        }
        if ws > 0.0 {
            s.map(|v| v / ws)
        } else {
            src.px[y * w + x]
        }
    }))
}

pub(super) fn denoise(_n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (src.w, src.h);
    let normal = ins[1].realize(w, h).to_image("Normal");
    let albedo = if ins[2].image().is_some() { ins[2].realize(w, h).to_image("Albedo") } else { Buf::new(w, h, [1.0; 4]).to_image("Albedo") };
    let settings = crate::render::DenoiseSettings { threads: t, ..Default::default() };
    let out = crate::render::denoise_images(&src.to_image("Image"), &albedo, &normal, None, &settings);
    Val::img(Buf::from_image(&out))
}

pub(super) fn despeckle(n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[1].image() else { return ins[1].clone() };
    let (w, h) = (src.w, src.h);
    let fac = ins[0].realize(w, h);
    let th = pf(n, "threshold", 0.5);
    let thn = pf(n, "threshold_neighbor", 0.5);
    let diff = |a: Px, b: Px| (a[0] - b[0]).abs() > th || (a[1] - b[1]).abs() > th || (a[2] - b[2]).abs() > th;
    let cnr = std::f32::consts::FRAC_1_SQRT_2;
    let wtot = 4.0 + 4.0 * cnr;
    Val::img(gen(w, h, t, |x, y| {
        let org = src.px[y * w + x];
        let mut mid = [0.0; 4];
        let mut ok = [0.0; 4];
        let mut wt = 0.0;
        for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            let f = if dx != 0 && dy != 0 { cnr } else { 1.0 };
            let p = src.clamped(x as isize + dx, y as isize + dy);
            for c in 0..4 {
                mid[c] += p[c] * f;
            }
            if diff(p, org) {
                wt += f;
                for c in 0..4 {
                    ok[c] += p[c] * f;
                }
            }
        }
        let mid = mid.map(|v| v / wtot);
        if wt != 0.0 && wt / wtot > thn && diff(mid, org) {
            let ok = ok.map(|v| v / wt);
            let f = fac.px[y * w + x][0];
            [org[0] + (ok[0] - org[0]) * f, org[1] + (ok[1] - org[1]) * f, org[2] + (ok[2] - org[2]) * f, org[3] + (ok[3] - org[3]) * f]
        } else {
            org
        }
    }))
}

/// Squared Euclidean distance transform (Felzenszwalb) to the nearest seed,
/// with the index of that seed.
pub(super) fn edt(seeds: &[bool], w: usize, h: usize) -> (Vec<f32>, Vec<usize>) {
    const INF: f32 = 1e20;
    fn pass(f: &[f32], d: &mut [f32], arg: &mut [usize]) {
        let n = f.len();
        let mut v = vec![0usize; n];
        let mut z = vec![0f32; n + 1];
        let mut k = 0usize;
        z[0] = -INF;
        z[1] = INF;
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f32) - (f[p] + (p * p) as f32)) / (2.0 * q as f32 - 2.0 * p as f32);
                if s <= z[k] && k > 0 {
                    k -= 1;
                    continue;
                }
                if s <= z[k] {
                    v[0] = q;
                    z[0] = -INF;
                    z[1] = INF;
                    break;
                }
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = INF;
                break;
            }
        }
        k = 0;
        for q in 0..n {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let p = v[k];
            d[q] = (q as f32 - p as f32).powi(2) + f[p];
            arg[q] = p;
        }
    }
    let mut col_d = vec![INF; w * h];
    let mut col_arg = vec![0usize; w * h];
    let mut f = vec![0f32; h];
    let mut d = vec![0f32; h];
    let mut a = vec![0usize; h];
    for x in 0..w {
        for y in 0..h {
            f[y] = if seeds[y * w + x] { 0.0 } else { INF };
        }
        pass(&f, &mut d, &mut a);
        for y in 0..h {
            col_d[y * w + x] = d[y];
            col_arg[y * w + x] = a[y];
        }
    }
    let mut out_d = vec![INF; w * h];
    let mut out_i = vec![0usize; w * h];
    let mut f = vec![0f32; w];
    let mut d = vec![0f32; w];
    let mut a = vec![0usize; w];
    for y in 0..h {
        f.copy_from_slice(&col_d[y * w..(y + 1) * w]);
        pass(&f, &mut d, &mut a);
        for x in 0..w {
            out_d[y * w + x] = d[x];
            let px = a[x];
            out_i[y * w + x] = col_arg[y * w + px] * w + px;
        }
    }
    (out_d, out_i)
}

fn falloff(kind: &str, x: f32) -> f32 {
    let t = (1.0 - x).clamp(0.0, 1.0);
    match kind {
        "SPHERE" => (1.0 - x * x).max(0.0).sqrt(),
        "ROOT" => t.sqrt(),
        "INVERSE_SQUARE" => t * (2.0 - t),
        "SHARP" => t * t,
        "LINEAR" => t,
        _ => t * t * (3.0 - 2.0 * t),
    }
}

pub(super) fn dilate_erode(n: &Node, v: &Val, t: usize) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let (w, h) = (src.w, src.h);
    let d = pi(n, "distance", 0);
    if d == 0 {
        return v.clone();
    }
    let r = d.unsigned_abs() as isize;
    let dilate = d > 0;
    match pe(n, "mode") {
        "STEP" => {
            // Square structuring element, separable max/min.
            let pick = |a: f32, b: f32| if dilate { a.max(b) } else { a.min(b) };
            let mut tmp = Buf::new(w, h, [0.0; 4]);
            par_rows(&mut tmp.px, w, t, |y, row| {
                for (x, o) in row.iter_mut().enumerate() {
                    let mut m = src.px[y * w + x][0];
                    for dx in -r..=r {
                        m = pick(m, src.clamped(x as isize + dx, y as isize)[0]);
                    }
                    *o = f1(m);
                }
            });
            Val::img(gen(w, h, t, |x, y| {
                let mut m = tmp.px[y * w + x][0];
                for dy in -r..=r {
                    m = pick(m, tmp.clamped(x as isize, y as isize + dy)[0]);
                }
                f1(m)
            }))
        }
        "FEATHER" => {
            let kind = pe(n, "falloff").to_string();
            let rf = r as f32;
            Val::img(gen(w, h, t, |x, y| {
                let mut m = if dilate { 0.0f32 } else { 1.0f32 };
                for dy in -r..=r {
                    for dx in -r..=r {
                        let dist = ((dx * dx + dy * dy) as f32).sqrt();
                        if dist > rf {
                            continue;
                        }
                        let fo = falloff(&kind, dist / rf);
                        let s = src.clamped(x as isize + dx, y as isize + dy)[0];
                        if dilate {
                            m = m.max(s * fo);
                        } else {
                            m = m.min(1.0 - (1.0 - s) * fo);
                        }
                    }
                }
                f1(m)
            }))
        }
        mode => {
            let edge = if mode == "THRESHOLD" { pf(n, "edge", 0.0) } else { 0.0 };
            let inside: Vec<bool> = src.px.iter().map(|p| p[0] > 0.5).collect();
            let seeds: Vec<bool> = if dilate { inside.clone() } else { inside.iter().map(|b| !b).collect() };
            let (dist, _) = edt(&seeds, w, h);
            let rf = r as f32;
            Val::img(Buf {
                w,
                h,
                px: dist
                    .iter()
                    .map(|d2| {
                        let d = d2.sqrt();
                        let v = if edge > 0.0 { ((rf - d) / edge + 0.5).clamp(0.0, 1.0) } else if d <= rf { 1.0 } else { 0.0 };
                        f1(if dilate { v } else { 1.0 - v })
                    })
                    .collect(),
            })
        }
    }
}

pub(super) fn directional_blur(n: &Node, v: &Val, t: usize) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let (w, h) = (src.w, src.h);
    let iters = pi(n, "iterations", 1).clamp(1, 32);
    let steps = 1usize << iters.min(16);
    let itsc = 1.0 / steps as f32;
    let diag = ((w * w + h * h) as f32).sqrt();
    let dist = pf(n, "distance", 0.0) * diag;
    let a = pf(n, "angle", 0.0);
    let (cx, cy) = (pf(n, "center_x", 0.5) * w as f32, (1.0 - pf(n, "center_y", 0.5)) * h as f32);
    let tx = itsc * dist * a.cos();
    let ty = itsc * dist * a.sin();
    let sc = itsc * pf(n, "zoom", 0.0);
    let rot = itsc * pf(n, "spin", 0.0);
    if dist == 0.0 && sc == 0.0 && rot == 0.0 {
        return v.clone();
    }
    let src = src.clone();
    Val::img(gen(w, h, t, |x, y| {
        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut s = [0.0; 4];
        for i in 0..steps {
            let fi = i as f32;
            let (ss, cs) = (fi * rot).sin_cos();
            let isc = 1.0 / (1.0 + fi * sc);
            let u = isc * (px - cx) - fi * tx;
            let vv = isc * (py - cy) + fi * ty;
            let p = src.bilinear_clamped(cs * u + ss * vv + cx, cs * vv - ss * u + cy);
            for c in 0..4 {
                s[c] += p[c];
            }
        }
        s.map(|v| v / steps as f32)
    }))
}

/// The Filter node's 3x3 kernels; edge filters return the gradient
/// magnitude.
pub(super) fn filter3x3(kind: &str, fac: &Val, v: &Val, t: usize) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let (w, h) = (src.w, src.h);
    let fac = fac.realize(w, h);
    let k: [f32; 9] = match kind {
        "SHARPEN" => [-1.0, -1.0, -1.0, -1.0, 9.0, -1.0, -1.0, -1.0, -1.0],
        "SHARPEN_DIAMOND" => [0.0, -1.0, 0.0, -1.0, 5.0, -1.0, 0.0, -1.0, 0.0],
        "LAPLACE" => [-0.125, -0.125, -0.125, -0.125, 1.0, -0.125, -0.125, -0.125, -0.125],
        "SOBEL" => [1.0, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, -2.0, -1.0],
        "PREWITT" => [1.0, 1.0, 1.0, 0.0, 0.0, 0.0, -1.0, -1.0, -1.0],
        "KIRSCH" => [5.0, 5.0, 5.0, -3.0, 0.0, -3.0, -3.0, -3.0, -3.0],
        "SHADOW" => [1.0, 2.0, 1.0, 0.0, 1.0, 0.0, -1.0, -2.0, -1.0],
        _ => [1.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0, 2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0, 2.0 / 16.0, 1.0 / 16.0],
    };
    let edge = matches!(kind, "SOBEL" | "PREWITT" | "KIRSCH");
    let src = src.clone();
    Val::img(gen(w, h, t, |x, y| {
        let mut a = [0.0; 3];
        let mut b = [0.0; 3];
        for j in 0..3 {
            for i in 0..3 {
                let p = src.clamped(x as isize + i as isize - 1, y as isize + j as isize - 1);
                for c in 0..3 {
                    a[c] += p[c] * k[j * 3 + i];
                    // Transposed kernel for the second gradient direction.
                    b[c] += p[c] * k[i * 3 + j];
                }
            }
        }
        let org = src.px[y * w + x];
        let r = if edge { [(a[0] * a[0] + b[0] * b[0]).sqrt(), (a[1] * a[1] + b[1] * b[1]).sqrt(), (a[2] * a[2] + b[2] * b[2]).sqrt()] } else { a };
        let f = fac.px[y * w + x][0];
        [org[0] + (r[0] - org[0]) * f, org[1] + (r[1] - org[1]) * f, org[2] + (r[2] - org[2]) * f, org[3]]
    }))
}

/// Inpaint: fill transparent pixels within `distance` of opaque ones with
/// the nearest opaque colour (premultiplied over).
pub(super) fn inpaint(distance: i32, v: &Val) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    if distance <= 0 {
        return v.clone();
    }
    let (w, h) = (src.w, src.h);
    let seeds: Vec<bool> = src.px.iter().map(|p| p[3] >= 1.0).collect();
    if !seeds.iter().any(|&b| b) {
        return v.clone();
    }
    let (d, near) = edt(&seeds, w, h);
    let lim = (distance as f32).powi(2);
    let px = src
        .px
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if p[3] >= 1.0 || d[i] > lim {
                *p
            } else {
                let q = src.px[near[i]];
                let m = 1.0 - p[3];
                [p[0] + q[0] * m, p[1] + q[1] * m, p[2] + q[2] * m, p[3] + q[3] * m]
            }
        })
        .collect();
    Val::img(Buf { w, h, px })
}

pub(super) fn pixelate(size: usize, v: &Val) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    if size <= 1 {
        return v.clone();
    }
    let (w, h) = (src.w, src.h);
    let mut out = Buf::new(w, h, [0.0; 4]);
    for by in (0..h).step_by(size) {
        for bx in (0..w).step_by(size) {
            let mut s = [0.0; 4];
            let mut n = 0.0;
            for y in by..(by + size).min(h) {
                for x in bx..(bx + size).min(w) {
                    let p = src.px[y * w + x];
                    for c in 0..4 {
                        s[c] += p[c];
                    }
                    n += 1.0;
                }
            }
            let s = s.map(|v| v / n);
            for y in by..(by + size).min(h) {
                for x in bx..(bx + size).min(w) {
                    out.px[y * w + x] = s;
                }
            }
        }
    }
    Val::img(out)
}

pub(super) fn sun_beams(n: &Node, v: &Val, t: usize) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let (w, h) = (src.w, src.h);
    let source = match n.prop("source") {
        Some(crate::nodes::Prop::Vector(s)) => [s[0], s[1]],
        _ => [0.5, 0.5],
    };
    let ray = pf(n, "ray_length", 0.0);
    if ray <= 0.0 {
        return v.clone();
    }
    let max_dim = w.max(h) as f32;
    let src = src.clone();
    Val::img(gen(w, h, t, |x, y| {
        let c = [(x as f32 + 0.5) / w as f32, 1.0 - (y as f32 + 0.5) / h as f32];
        let to = [source[0] - c[0], source[1] - c[1]];
        let dist = (to[0] * to[0] + to[1] * to[1]).sqrt();
        if dist < 1e-6 {
            return src.px[y * w + x];
        }
        let len = dist.min(ray);
        let steps = ((len * max_dim) as usize).max(1);
        let mut s = [0.0; 4];
        let mut ws = 0.0;
        for i in 0..steps {
            let f = i as f32 / steps as f32;
            let p = [c[0] + to[0] / dist * len * f, c[1] + to[1] / dist * len * f];
            let k = 1.0 - (len * f) / ray;
            let q = src.bilinear_clamped(p[0] * w as f32, (1.0 - p[1]) * h as f32);
            for ch in 0..4 {
                s[ch] += q[ch] * k;
            }
            ws += 1.0;
        }
        s.map(|v| v / ws)
    }))
}

/// Classic Kuwahara: the mean of the quadrant with the lowest luminance
/// variance (the anisotropic variant uses the same).
pub(super) fn kuwahara(_n: &Node, ins: &[Val], t: usize) -> Val {
    let Some(src) = ins[0].image() else { return ins[0].clone() };
    let (w, h) = (src.w, src.h);
    let r = ins[1].as_single()[0].max(0.0) as isize;
    if r < 1 {
        return ins[0].clone();
    }
    let src = src.clone();
    Val::img(gen(w, h, t, |x, y| {
        let mut best = (f32::MAX, [0.0; 4]);
        for (qx, qy) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            let mut s = [0.0; 4];
            let mut l2 = 0.0;
            let mut l1 = 0.0;
            let mut n = 0.0;
            for j in 0..=r {
                for i in 0..=r {
                    let p = src.clamped(x as isize + i * qx, y as isize + j * qy);
                    let l = luminance([p[0], p[1], p[2]]);
                    for c in 0..4 {
                        s[c] += p[c];
                    }
                    l1 += l;
                    l2 += l * l;
                    n += 1.0;
                }
            }
            let var = l2 / n - (l1 / n).powi(2);
            if var < best.0 {
                best = (var, s.map(|v| v / n));
            }
        }
        best.1
    }))
}

/// Edge anti-aliasing: pixels whose 3x3 luminance contrast exceeds
/// `threshold` blend toward their neighbourhood average.
pub(super) fn antialias(v: &Val, threshold: f32, t: usize) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let (w, h) = (src.w, src.h);
    let src = src.clone();
    let th = threshold.max(1e-4);
    Val::img(gen(w, h, t, |x, y| {
        let c = src.px[y * w + x];
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut avg = [0.0; 4];
        for dy in -1..=1 {
            for dx in -1..=1 {
                let p = src.clamped(x as isize + dx, y as isize + dy);
                let l = luminance([p[0], p[1], p[2]]).max(p[3] * 0.5);
                lo = lo.min(l);
                hi = hi.max(l);
                let k = if dx == 0 && dy == 0 { 4.0 } else if dx == 0 || dy == 0 { 2.0 } else { 1.0 };
                for ch in 0..4 {
                    avg[ch] += p[ch] * k / 16.0;
                }
            }
        }
        let contrast = hi - lo;
        if contrast < th {
            return c;
        }
        let f = (contrast / th).min(1.0) * 0.75;
        [c[0] + (avg[0] - c[0]) * f, c[1] + (avg[1] - c[1]) * f, c[2] + (avg[2] - c[2]) * f, c[3] + (avg[3] - c[3]) * f]
    }))
}

pub(super) fn normalize(v: &Val) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for p in &src.px {
        if p[0].abs() <= 10000.0 {
            lo = lo.min(p[0]);
            hi = hi.max(p[0]);
        }
    }
    let range = hi - lo;
    Val::img(Buf { w: src.w, h: src.h, px: src.px.iter().map(|p| f1(if range != 0.0 && lo <= hi { (p[0] - lo) / range } else { p[0] })).collect() })
}

pub(super) fn levels(n: &Node, v: &Val) -> Vec<Val> {
    let ch = pe(n, "channel").to_string();
    let get = |p: &Px| match ch.as_str() {
        "RED" => p[0],
        "GREEN" => p[1],
        "BLUE" => p[2],
        "LUMINANCE" => luminance([p[0], p[1], p[2]]),
        _ => (p[0] + p[1] + p[2]) / 3.0,
    };
    let px: Vec<Px> = match v {
        Val::Single(p) => vec![*p],
        Val::Image(b) => b.px.clone(),
    };
    let nn = px.len().max(1) as f32;
    let mean = px.iter().map(get).sum::<f32>() / nn;
    let var = px.iter().map(|p| (get(p) - mean).powi(2)).sum::<f32>() / nn;
    vec![Val::float(mean), Val::float(var.sqrt())]
}

/// Tonemap: Reinhard simple (key/offset/gamma) and R/D Photoreceptor.
pub(super) fn tonemap(n: &Node, v: &Val, t: usize) -> Val {
    let px: Vec<Px> = match v {
        Val::Single(p) => vec![*p],
        Val::Image(b) => b.px.clone(),
    };
    let np = px.len().max(1) as f32;
    let lum = |p: &Px| luminance([p[0], p[1], p[2]]);
    let map: Box<dyn Fn(Px) -> Px + Sync> = if pe(n, "tonemap_type") == "RH_SIMPLE" {
        let lsum: f32 = px.iter().map(|p| (lum(p).max(0.0) + 1e-5).ln()).sum();
        let geo = (lsum / np).exp();
        let key = pf(n, "key", 0.18);
        let scale = if geo != 0.0 { key / geo } else { 0.0 };
        let off = pf(n, "offset", 1.0);
        let ig = 1.0 / pf(n, "gamma", 1.0).max(1e-6);
        Box::new(move |p: Px| {
            let mut o = p;
            for c in 0..3 {
                let s = p[c] * scale;
                let d = s + off;
                let v = if d != 0.0 { s / d } else { 0.0 };
                o[c] = if v > 0.0 { v.powf(ig) } else { v };
            }
            o
        })
    } else {
        let (mut lsum, mut lav, mut maxl, mut minl) = (0.0f32, 0.0f32, f32::MIN, f32::MAX);
        let mut cav = [0.0f32; 3];
        for p in &px {
            let l = lum(p);
            lav += l;
            for c in 0..3 {
                cav[c] += p[c];
            }
            let ll = (l.max(0.0) + 1e-5).ln();
            lsum += ll;
            maxl = maxl.max(ll);
            minl = minl.min(ll);
        }
        lav /= np;
        let cav = cav.map(|v| v / np);
        let avl = lsum / np;
        let auto_key = if maxl > minl { (maxl - avl) / (maxl - minl) } else { 1.0 };
        let f = (-pf(n, "intensity", 0.0)).exp();
        let m = {
            let m = pf(n, "contrast", 0.0);
            if m > 0.0 {
                m
            } else {
                0.3 + 0.7 * auto_key.powf(1.4)
            }
        };
        let ic = 1.0 - pf(n, "correction", 0.0);
        let ia = 1.0 - pf(n, "adaptation", 1.0);
        Box::new(move |p: Px| {
            let l = luminance([p[0], p[1], p[2]]);
            let mut o = p;
            for c in 0..3 {
                let il = p[c] + ic * (l - p[c]);
                let ig = cav[c] + ic * (lav - cav[c]);
                let ia_ = il + ia * (ig - il);
                let d = p[c] + (f * ia_).max(0.0).powf(m);
                o[c] = if d != 0.0 { p[c] / d } else { 0.0 };
            }
            o
        })
    };
    match v {
        Val::Single(p) => Val::Single(map(*p)),
        Val::Image(b) => {
            let mut o = (**b).clone();
            par_rows(&mut o.px, b.w, t, |_, row| {
                for p in row {
                    *p = map(*p);
                }
            });
            Val::img(o)
        }
    }
}

/// Keying: screen-colour matte with pre-blur, clipping (except on edges),
/// garbage/core mattes, despill and post-blur; outputs Image, Matte, Edges.
pub(super) fn keying(n: &Node, ins: &[Val], t: usize) -> Vec<Val> {
    let Some((w, h)) = super::domain(ins, &[0]) else {
        return vec![ins[0].clone(), Val::float(1.0), Val::float(0.0)];
    };
    let mut img = (*ins[0].realize(w, h)).clone();
    let key = ins[1].realize(w, h);
    let garbage = ins[2].realize(w, h);
    let core = ins[3].realize(w, h);
    let screen_balance = pf(n, "screen_balance", 0.5);
    let blur_pre = pi(n, "blur_pre", 0);
    let argmax = |c: Px| -> [usize; 3] {
        let mut i = [0usize, 1, 2];
        i.sort_by(|a, b| c[*b].total_cmp(&c[*a]));
        i
    };
    let saturation = |c: Px, am: [usize; 3], bal: f32| {
        let wa = c[am[1]] * bal + c[am[2]] * (1.0 - bal);
        (c[am[0]] - wa) * (1.0 - wa).abs()
    };
    let chroma_src = if blur_pre > 0 { gaussian(&img, blur_pre as f32, t) } else { img.clone() };
    let mut matte: Vec<f32> = (0..w * h)
        .map(|i| {
            let k = key.px[i];
            let am = argmax(k);
            let ks = saturation(k, am, screen_balance);
            let cs = saturation(chroma_src.px[i], am, screen_balance);
            if ks.abs() < 1e-9 {
                1.0
            } else {
                1.0 - (cs / ks).clamp(0.0, 1.0)
            }
        })
        .collect();
    let radius = pi(n, "edge_kernel_radius", 3).max(0) as isize;
    let tol = pf(n, "edge_kernel_tolerance", 0.1);
    let (black, white) = (pf(n, "clip_black", 0.0), pf(n, "clip_white", 1.0));
    let mut edges = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let m = matte[y * w + x];
            let mut cnt = 0;
            let mut tot = 0;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (qx, qy) = ((x as isize + dx).clamp(0, w as isize - 1) as usize, (y as isize + dy).clamp(0, h as isize - 1) as usize);
                    if (matte[qy * w + qx] - m).abs() > tol {
                        cnt += 1;
                    }
                    tot += 1;
                }
            }
            edges[y * w + x] = if cnt as f32 >= 0.1 * tot as f32 { 1.0 } else { 0.0 };
        }
    }
    for i in 0..w * h {
        if edges[i] == 0.0 && (black > 0.0 || white < 1.0) {
            matte[i] = if white > black { ((matte[i] - black) / (white - black)).clamp(0.0, 1.0) } else { (matte[i] > black) as i32 as f32 };
        }
        matte[i] *= 1.0 - garbage.px[i][0];
        matte[i] = matte[i].max(core.px[i][0]);
    }
    let dilate = pi(n, "dilate_distance", 0);
    if dilate != 0 {
        let mut dn = n.clone();
        dn.set_prop("mode", crate::nodes::Prop::Enum("STEP".into()));
        dn.set_prop("distance", crate::nodes::Prop::Int(dilate));
        if let Some(b) = dilate_erode(&dn, &Val::img(Buf { w, h, px: matte.iter().map(|v| f1(*v)).collect() }), t).image() {
            matte = b.px.iter().map(|p| p[0]).collect();
        }
    }
    let blur_post = pi(n, "blur_post", 0);
    if blur_post > 0 {
        let b = gaussian(&Buf { w, h, px: matte.iter().map(|v| f1(*v)).collect() }, blur_post as f32, t);
        matte = b.px.iter().map(|p| p[0]).collect();
    }
    let despill = pf(n, "despill_factor", 1.0);
    let dbal = pf(n, "despill_balance", 0.5);
    for (i, p) in img.px.iter_mut().enumerate() {
        let am = argmax(key.px[i]);
        let wa = p[am[1]] * dbal + p[am[2]] * (1.0 - dbal);
        let amount = (p[am[0]] - wa) * despill;
        if amount > 0.0 {
            p[am[0]] -= amount;
        }
        let m = matte[i];
        *p = p.map(|v| v * m);
    }
    vec![Val::img(img), Val::img(Buf { w, h, px: matte.iter().map(|v| f1(*v)).collect() }), Val::img(Buf { w, h, px: edges.iter().map(|v| f1(*v)).collect() })]
}

/// Double Edge Mask: a gradient from the inner mask's edge (1) to the
/// outer mask's edge (0).
pub(super) fn double_edge_mask(n: &Node, ins: &[Val]) -> Val {
    let Some((w, h)) = super::domain(ins, &[]) else {
        return Val::float(if ins[0].as_single()[0] > 0.0 { 1.0 } else { 0.0 });
    };
    let inner = ins[0].realize(w, h);
    let outer = ins[1].realize(w, h);
    let adjacent_only = pe(n, "inner_mode") == "ADJACENT_ONLY";
    let keep_in = pe(n, "edge_mode") == "KEEP_IN";
    let iv = |x: isize, y: isize| inner.get(x, y)[0] > 0.0;
    let ov = |x: isize, y: isize| outer.get(x, y)[0] > 0.0;
    let mut inner_edge = vec![false; w * h];
    let mut outer_edge = vec![false; w * h];
    for y in 0..h as isize {
        for x in 0..w as isize {
            let i = y as usize * w + x as usize;
            let nb = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)];
            let border = nb.iter().any(|&(a, b)| a < 0 || b < 0 || a >= w as isize || b >= h as isize);
            if iv(x, y) {
                inner_edge[i] = nb.iter().any(|&(a, b)| {
                    let inside_img = a >= 0 && b >= 0 && a < w as isize && b < h as isize;
                    inside_img && !iv(a, b) && (!adjacent_only || ov(a, b))
                });
            } else if ov(x, y) {
                outer_edge[i] = nb.iter().any(|&(a, b)| a >= 0 && b >= 0 && a < w as isize && b < h as isize && !ov(a, b)) || (keep_in && border);
            }
        }
    }
    let has_inner = inner_edge.iter().any(|&b| b);
    let has_outer = outer_edge.iter().any(|&b| b);
    let (di, _) = edt(&inner_edge, w, h);
    let (dout, _) = edt(&outer_edge, w, h);
    let px = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let v = if iv(x, y) {
                1.0
            } else if ov(x, y) && has_inner {
                if !has_outer {
                    1.0
                } else {
                    let (a, b) = (di[i].sqrt(), dout[i].sqrt());
                    if a + b > 0.0 {
                        b / (a + b)
                    } else {
                        0.0
                    }
                }
            } else {
                0.0
            };
            f1(v)
        })
        .collect();
    Val::img(Buf { w, h, px })
}
