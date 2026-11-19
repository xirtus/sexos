// SPDX-License-Identifier: GPL-3.0-only
//! Glare, after Blender 4.2's realtime compositor (`node_composite_glare.cc`
//! and the `compositor_glare_*` shaders): highlights above the threshold
//! (HSV value minus threshold) at 1/1, 1/2 or 1/4 resolution, one of Bloom,
//! Fog Glow (FFT convolution), Streaks, Ghosts or Simple Star, then the
//! glare mixed back over the input.
//!
//! Glare buffers use Blender's bottom-left texel space (`at`/`put` flip
//! rows); normalized sampling is bilinear with zero outside unless noted.
use super::filter::gaussian;
use super::ops::{pb, pe, pf, pi};
use super::{gen, par_rows, Buf, Px, Val};
use crate::nodes::Node;
use crate::render::shader::color::{hsv_to_rgb, rgb_to_hsv};
use std::f32::consts::PI;

/// Bilinear sample at normalized coordinates (y up), zero outside.
fn tex(b: &Buf, u: f32, v: f32) -> Px {
    b.bilinear(u * b.w as f32, (1.0 - v) * b.h as f32)
}
/// Bilinear sample at normalized coordinates (y up), clamped to the edge.
fn tex_clamped(b: &Buf, u: f32, v: f32) -> Px {
    b.bilinear_clamped(u * b.w as f32, (1.0 - v) * b.h as f32)
}
/// Pixel at bottom-left texel (x, y), zero outside.
fn at(b: &Buf, x: isize, y: isize) -> Px {
    b.get(x, b.h as isize - 1 - y)
}
fn add(a: Px, b: Px) -> Px {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}
fn mul(a: Px, s: f32) -> Px {
    a.map(|v| v * s)
}
fn mix(a: Px, b: Px, t: f32) -> Px {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// `gen` in bottom-left texel space: `f(x, y)` with y up.
fn gen_up(w: usize, h: usize, t: usize, f: impl Fn(usize, usize) -> Px + Sync) -> Buf {
    gen(w, h, t, |x, y| f(x, h - 1 - y))
}

pub(super) fn glare(n: &Node, v: &Val, t: usize) -> Val {
    let Some(src) = v.image() else { return v.clone() };
    let mix_factor = pf(n, "mix", 0.0);
    if mix_factor == -1.0 {
        return v.clone();
    }
    let qf = match pe(n, "quality") {
        "HIGH" => 1,
        "LOW" => 4,
        _ => 2,
    };
    let (gw, gh) = ((src.w / qf).max(1), (src.h / qf).max(1));
    let threshold = pf(n, "threshold", 1.0);
    let highlights = gen_up(gw, gh, t, |x, y| {
        let c = tex_clamped(src, (x as f32 + 0.5) / gw as f32, (y as f32 + 0.5) / gh as f32);
        let mut hsv = rgb_to_hsv([c[0], c[1], c[2]]);
        hsv[2] = (hsv[2] - threshold).max(0.0);
        let r = hsv_to_rgb(hsv);
        [r[0], r[1], r[2], 1.0]
    });
    let iterations = pi(n, "iterations", 3).clamp(2, 5) as usize;
    let colmod = pf(n, "color_modulation", 0.25);
    let fade = pf(n, "fade", 0.9);
    let glare = match pe(n, "glare_type") {
        "BLOOM" => bloom(&highlights, pi(n, "size", 8), t),
        "FOG_GLOW" => fog_glow(&highlights, pi(n, "size", 8)),
        "GHOSTS" => ghosts(&highlights, iterations, colmod, qf, t),
        "SIMPLE_STAR" => simple_star(&highlights, iterations, fade, pb(n, "use_rotate_45")),
        _ => streaks(&highlights, iterations, colmod, fade, pi(n, "streaks", 4).max(1) as usize, pf(n, "angle_offset", 0.0), t),
    };
    let iw = 1.0 - mix_factor.max(0.0);
    let gw_ = 1.0 + mix_factor.min(0.0);
    let (w, h) = (src.w, src.h);
    Val::img(gen_up(w, h, t, |x, y| {
        let g = tex_clamped(&glare, (x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
        let i = at(src, x as isize, y as isize).map(|v| v.max(0.0));
        [iw * i[0] + gw_ * g[0], iw * i[1] + gw_ * g[1], iw * i[2] + gw_ * g[2], i[3]]
    }))
}

// ---------------------------------------------------------------------------
// Bloom

fn karis_weight(c: Px) -> f32 {
    1.0 / (1.0 + c[0].max(c[1]).max(c[2]))
}

fn downsample(b: &Buf, karis: bool, t: usize) -> Buf {
    let (w, h) = ((b.w / 2).max(1), (b.h / 2).max(1));
    gen_up(w, h, t, |x, y| {
        let c = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let ps = [1.0 / b.w as f32, 1.0 / b.h as f32];
        let s = |dx: f32, dy: f32| tex_clamped(b, c[0] + dx * ps[0], c[1] + dy * ps[1]);
        let (a, bb, cc) = (s(-2.0, 2.0), s(0.0, 2.0), s(2.0, 2.0));
        let (d, e, f) = (s(-2.0, 0.0), s(0.0, 0.0), s(2.0, 0.0));
        let (g, hh, i) = (s(-2.0, -2.0), s(0.0, -2.0), s(2.0, -2.0));
        let (j, k, l, m) = (s(-1.0, 1.0), s(1.0, 1.0), s(-1.0, -1.0), s(1.0, -1.0));
        if karis {
            let boxes = [
                (mul(add(add(j, k), add(l, m)), 0.25), 0.5),
                (mul(add(add(a, bb), add(d, e)), 0.25), 0.125),
                (mul(add(add(bb, cc), add(e, f)), 0.25), 0.125),
                (mul(add(add(d, e), add(g, hh)), 0.25), 0.125),
                (mul(add(add(e, f), add(hh, i)), 0.25), 0.125),
            ];
            let mut sum = [0.0; 4];
            let mut ws = 0.0;
            for (bx, wt) in boxes {
                let k = karis_weight(bx) * wt;
                sum = add(sum, mul(bx, k));
                ws += k;
            }
            mul(sum, 1.0 / ws)
        } else {
            let mut r = mul(e, 4.0 / 32.0);
            r = add(r, mul(add(add(a, cc), add(g, i)), 1.0 / 32.0));
            r = add(r, mul(add(add(bb, d), add(f, hh)), 2.0 / 32.0));
            r = add(r, mul(add(add(j, k), add(l, m)), 4.0 / 32.0));
            r
        }
    })
}

fn upsample_add(small: &Buf, big: &mut Buf, t: usize) {
    let (w, h) = (big.w, big.h);
    let add_buf = gen_up(w, h, t, |x, y| {
        let c = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let ps = [1.0 / w as f32, 1.0 / h as f32];
        let s = |dx: f32, dy: f32| tex_clamped(small, c[0] + dx * ps[0], c[1] + dy * ps[1]);
        let mut u = mul(s(0.0, 0.0), 4.0 / 16.0);
        for (dx, dy) in [(-1.0, 0.0), (0.0, 1.0), (1.0, 0.0), (0.0, -1.0)] {
            u = add(u, mul(s(dx, dy), 2.0 / 16.0));
        }
        for (dx, dy) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            u = add(u, mul(s(dx, dy), 1.0 / 16.0));
        }
        u
    });
    for (p, a) in big.px.iter_mut().zip(add_buf.px) {
        *p = add(*p, a);
    }
}

fn bloom(highlights: &Buf, size: i32, t: usize) -> Buf {
    let smaller = highlights.w.min(highlights.h).max(1);
    let max_chain = (smaller as f32).log2().floor() as i32;
    let chain_length = max_chain - (9 - size.clamp(1, 9));
    if chain_length < 2 {
        return Buf::new(highlights.w, highlights.h, [0.0; 4]);
    }
    let mut chain = vec![highlights.clone()];
    for i in 1..chain_length as usize {
        let next = downsample(&chain[i - 1], i == 1, t);
        chain.push(next);
    }
    for i in (1..chain.len()).rev() {
        let (lo, hi) = chain.split_at_mut(i);
        upsample_add(&hi[0], &mut lo[i - 1], t);
    }
    chain.swap_remove(0)
}

// ---------------------------------------------------------------------------
// Streaks

fn streaks(highlights: &Buf, iterations: usize, colmod: f32, fade: f32, count: usize, angle_offset: f32, t: usize) -> Buf {
    let (w, h) = (highlights.w, highlights.h);
    let mut acc = Buf::new(w, h, [0.0; 4]);
    let attenuation = 1.0 / count as f32;
    for s in 0..count {
        let angle = angle_offset + s as f32 / count as f32 * 2.0 * PI;
        let dir = [angle.cos(), angle.sin()];
        let mut cur = highlights.clone();
        for it in 0..iterations {
            let cm = 1.0 - colmod.powi(it as i32 + 1);
            let mag = 4f32.powi(it as i32);
            let f = fade.powf(mag);
            let fades = [f, f * f, f * f * f];
            let vec = [dir[0] * mag / w as f32, dir[1] * mag / h as f32];
            let input = cur;
            cur = gen_up(w, h, t, |x, y| {
                let c = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
                let mut nb = [[0.0f32; 4]; 3];
                for (k, n) in nb.iter_mut().enumerate() {
                    let m = (k + 1) as f32;
                    *n = tex(&input, c[0] + vec[0] * m, c[1] + vec[1] * m);
                }
                nb[0][1] *= cm;
                nb[0][2] *= cm;
                nb[1][0] *= cm;
                nb[1][1] *= cm;
                nb[2][0] *= cm;
                nb[2][2] *= cm;
                let mut sum = [0.0; 4];
                for k in 0..3 {
                    sum = add(sum, mul(nb[k], fades[k]));
                }
                mul(add(tex(&input, c[0], c[1]), sum), 0.5)
            });
        }
        for (a, p) in acc.px.iter_mut().zip(&cur.px) {
            *a = add(*a, mul(*p, attenuation));
        }
    }
    acc
}

// ---------------------------------------------------------------------------
// Ghosts

fn ghosts(highlights: &Buf, iterations: usize, colmod: f32, qf: usize, t: usize) -> Buf {
    let (w, h) = (highlights.w, highlights.h);
    let small_r = 16.0 / qf as f32;
    let small = gaussian(highlights, small_r, t);
    let big = gaussian(highlights, small_r * 2.0, t);
    let base = gen_up(w, h, t, |x, y| {
        let c = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
        let d = ((c[0] - 0.5).powi(2) + (c[1] - 0.5).powi(2)).sqrt() * 2.0;
        let (ss, bs) = (2.13f32, -0.97f32);
        let sa = (1.0 - d * ss).max(0.0);
        let ba = (1.0 - d * bs.abs()).max(0.0);
        let sg = tex(&small, (c[0] - 0.5) * ss + 0.5, (c[1] - 0.5) * ss + 0.5);
        let bg = tex(&big, (c[0] - 0.5) * bs + 0.5, (c[1] - 0.5) * bs + 0.5);
        add(mul(sg, sa), mul(bg, ba))
    });
    let c = 1.0 - colmod;
    let mods: [Px; 4] = [[1.0; 4], [1.0, c, c, 1.0], [c, c, 1.0, 1.0], [c, 1.0, c, 1.0]];
    let mut acc = Buf::new(w, h, [0.0; 4]);
    let mut input = base;
    let offset = if iterations % 2 == 1 { 0.5 } else { 0.0 };
    for i in 0..iterations {
        let mut scales = [0.0f32; 4];
        for (k, s) in scales.iter_mut().enumerate() {
            *s = 2.1 * (1.0 - (i as f32 * 4.0 + k as f32 + offset) / (iterations as f32 * 4.0));
            if k % 2 == 1 {
                *s = -0.99 / *s;
            }
        }
        let src = input.clone();
        let layer = gen_up(w, h, t, |x, y| {
            let cc = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
            let d = ((cc[0] - 0.5).powi(2) + (cc[1] - 0.5).powi(2)).sqrt() * 2.0;
            let mut a = [0.0; 4];
            for k in 0..4 {
                let s = scales[k];
                let att = (1.0 - d * s.abs()).max(0.0) / 4.0;
                let g = tex(&src, (cc[0] - 0.5) * s + 0.5, (cc[1] - 0.5) * s + 0.5);
                for ch in 0..4 {
                    a[ch] += g[ch] * att * mods[k][ch];
                }
            }
            a
        });
        for (p, l) in acc.px.iter_mut().zip(&layer.px) {
            *p = add(*p, *l);
        }
        input = acc.clone();
    }
    acc
}

// ---------------------------------------------------------------------------
// Simple Star

/// One recursive line filter (causal then non-causal per iteration) along
/// the texels `line` of `b` (bottom-left indices).
fn star_line(b: &mut Buf, line: &[(isize, isize)], dir: (isize, isize), iterations: usize, fade: f32) {
    let (w, h) = (b.w as isize, b.h as isize);
    let load = |b: &Buf, x: isize, y: isize| -> Px {
        if x < 0 || y < 0 || x >= w || y >= h {
            [0.0; 4]
        } else {
            b.px[((h - 1 - y) * w + x) as usize]
        }
    };
    for i in 0..iterations as isize {
        for &(x, y) in line {
            let prev = load(b, x - i * dir.0, y - i * dir.1);
            let cur = load(b, x, y);
            let next = load(b, x + i * dir.0, y + i * dir.1);
            let o = mix(cur, mul(add(prev, next), 0.5), fade);
            b.px[((h - 1 - y) * w + x) as usize] = o;
        }
        for &(x, y) in line.iter().rev() {
            let prev = load(b, x + i * dir.0, y + i * dir.1);
            let cur = load(b, x, y);
            let next = load(b, x - i * dir.0, y - i * dir.1);
            let o = mix(cur, mul(add(prev, next), 0.5), fade);
            b.px[((h - 1 - y) * w + x) as usize] = o;
        }
    }
}

fn simple_star(highlights: &Buf, iterations: usize, fade: f32, diagonal: bool) -> Buf {
    let (w, h) = (highlights.w as isize, highlights.h as isize);
    let mut a = highlights.clone();
    let mut b = highlights.clone();
    if diagonal {
        // Diagonals (1, 1) start on the left column and the bottom row;
        // anti-diagonals (1, -1) on the left column and the top row.
        let mut starts: Vec<(isize, isize)> = (0..h).rev().map(|y| (0, y)).collect();
        starts.extend((1..w).map(|x| (x, 0)));
        for (sx, sy) in starts {
            let line: Vec<(isize, isize)> = (0..).map(|j| (sx + j, sy + j)).take_while(|&(x, y)| x < w && y < h).collect();
            star_line(&mut a, &line, (1, 1), iterations, fade);
        }
        let mut starts: Vec<(isize, isize)> = (0..h).map(|y| (0, y)).collect();
        starts.extend((1..w).map(|x| (x, h - 1)));
        for (sx, sy) in starts {
            let line: Vec<(isize, isize)> = (0..).map(|j| (sx + j, sy - j)).take_while(|&(x, y)| x < w && y >= 0).collect();
            star_line(&mut b, &line, (1, -1), iterations, fade);
        }
    } else {
        for y in 0..h {
            let line: Vec<(isize, isize)> = (0..w).map(|x| (x, y)).collect();
            star_line(&mut a, &line, (1, 0), iterations, fade);
        }
        for x in 0..w {
            let line: Vec<(isize, isize)> = (0..h).map(|y| (x, y)).collect();
            star_line(&mut b, &line, (0, 1), iterations, fade);
        }
    }
    for (p, q) in b.px.iter_mut().zip(&a.px) {
        *p = add(*p, *q);
    }
    b
}

// ---------------------------------------------------------------------------
// Fog Glow (FFT convolution)

#[derive(Clone, Copy, Default)]
struct C {
    re: f32,
    im: f32,
}

impl C {
    fn mul(self, o: C) -> C {
        C { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }
}

/// In-place iterative radix-2 FFT of a power-of-two length.
fn fft(a: &mut [C], inverse: bool) {
    let n = a.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            a.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = 2.0 * std::f64::consts::PI / len as f64 * if inverse { 1.0 } else { -1.0 };
        let wl = C { re: ang.cos() as f32, im: ang.sin() as f32 };
        for i in (0..n).step_by(len) {
            let mut wv = C { re: 1.0, im: 0.0 };
            for k in 0..len / 2 {
                let u = a[i + k];
                let v = a[i + k + len / 2].mul(wv);
                a[i + k] = C { re: u.re + v.re, im: u.im + v.im };
                a[i + k + len / 2] = C { re: u.re - v.re, im: u.im - v.im };
                wv = wv.mul(wl);
            }
        }
        len <<= 1;
    }
    if inverse {
        let s = 1.0 / n as f32;
        for v in a.iter_mut() {
            v.re *= s;
            v.im *= s;
        }
    }
}

/// 2D FFT of a `w` x `h` (powers of two) row-major grid.
fn fft2d(a: &mut [C], w: usize, h: usize, inverse: bool) {
    par_rows(a, w, 0, |_, row| fft(row, inverse));
    let mut col = vec![C::default(); h];
    for x in 0..w {
        for y in 0..h {
            col[y] = a[y * w + x];
        }
        fft(&mut col, inverse);
        for y in 0..h {
            a[y * w + x] = col[y];
        }
    }
}

fn fog_kernel_value(x: usize, y: usize, ks: usize) -> f32 {
    let half = (ks / 2) as f32;
    let scale = 0.25 * ks as f32;
    let v = (y as f32 - half) / half;
    let u = (x as f32 - half) / half;
    let r = (u * u + v * v) * scale;
    let d = -r.sqrt().sqrt().sqrt() * 9.0;
    let window = (0.5 + 0.5 * (u * PI).cos()) * (0.5 + 0.5 * (v * PI).cos());
    window * d.exp()
}

fn fog_glow(highlights: &Buf, size: i32) -> Buf {
    let ks = (1usize << size.clamp(1, 10)) + 1;
    let (w, h) = (highlights.w, highlights.h);
    let fw = (w + ks).next_power_of_two();
    let fh = (h + ks).next_power_of_two();
    let half = ks / 2;
    let mut kernel = vec![C::default(); fw * fh];
    let mut sum = 0.0f64;
    for y in 0..ks {
        for x in 0..ks {
            let v = fog_kernel_value(x, y, ks);
            let ox = (x as isize - half as isize).rem_euclid(fw as isize) as usize;
            let oy = (y as isize - half as isize).rem_euclid(fh as isize) as usize;
            kernel[oy * fw + ox].re = v;
            sum += v as f64;
        }
    }
    fft2d(&mut kernel, fw, fh, false);
    let norm = 1.0 / sum.max(1e-12) as f32;
    let mut out = Buf::new(w, h, [0.0, 0.0, 0.0, 1.0]);
    for ch in 0..3 {
        let mut g = vec![C::default(); fw * fh];
        for y in 0..h {
            for x in 0..w {
                g[y * fw + x].re = highlights.px[y * w + x][ch];
            }
        }
        fft2d(&mut g, fw, fh, false);
        for (a, k) in g.iter_mut().zip(&kernel) {
            *a = a.mul(*k);
        }
        fft2d(&mut g, fw, fh, true);
        for y in 0..h {
            for x in 0..w {
                out.px[y * w + x][ch] = g[y * fw + x].re * norm;
            }
        }
    }
    out
}

#[cfg(test)]
pub(super) fn fft_roundtrip_error(n: usize) -> f32 {
    let mut a: Vec<C> = (0..n).map(|i| C { re: (i as f32 * 0.37).sin(), im: 0.0 }).collect();
    let orig = a.clone();
    fft(&mut a, false);
    fft(&mut a, true);
    a.iter().zip(&orig).map(|(x, y)| (x.re - y.re).abs() + x.im.abs()).fold(0.0, f32::max)
}
