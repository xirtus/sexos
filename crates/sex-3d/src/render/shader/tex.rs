// SPDX-License-Identifier: GPL-3.0-only
//! Procedural textures, ported from Cycles' kernel (`util/hash.h`,
//! `kernel/svm/noise.h`, `fractal_noise.h`, `voronoi.h`, `wave.h`, `magic.h`,
//! `brick.h`, `checker.h`, `gradient.h`, `white_noise.h`, `gabor.h`) so the
//! outputs match Blender's for the same inputs.
//!
//! Points are `[f32; N]` for N = 1..4 dimensions.
use core::f32::consts::{FRAC_PI_2, PI, TAU};

// ---------------------------------------------------------------------------
// Jenkins lookup3 hashes (Blender's `hash_uint*` / `hash_float*`)

#[inline]
fn rot(x: u32, k: u32) -> u32 {
    x.rotate_left(k)
}

#[inline]
fn final_mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *c ^= *b;
    *c = c.wrapping_sub(rot(*b, 14));
    *a ^= *c;
    *a = a.wrapping_sub(rot(*c, 11));
    *b ^= *a;
    *b = b.wrapping_sub(rot(*a, 25));
    *c ^= *b;
    *c = c.wrapping_sub(rot(*b, 16));
    *a ^= *c;
    *a = a.wrapping_sub(rot(*c, 4));
    *b ^= *a;
    *b = b.wrapping_sub(rot(*a, 14));
    *c ^= *b;
    *c = c.wrapping_sub(rot(*b, 24));
}

#[inline]
fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *a = a.wrapping_sub(*c);
    *a ^= rot(*c, 4);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= rot(*a, 6);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= rot(*b, 8);
    *b = b.wrapping_add(*a);
    *a = a.wrapping_sub(*c);
    *a ^= rot(*c, 16);
    *c = c.wrapping_add(*b);
    *b = b.wrapping_sub(*a);
    *b ^= rot(*a, 19);
    *a = a.wrapping_add(*c);
    *c = c.wrapping_sub(*b);
    *c ^= rot(*b, 4);
    *b = b.wrapping_add(*a);
}

pub fn hash_uint(kx: u32) -> u32 {
    let mut a = 0xdeadbeefu32.wrapping_add(1 << 2).wrapping_add(13);
    let (mut b, mut c) = (a, a);
    a = a.wrapping_add(kx);
    final_mix(&mut a, &mut b, &mut c);
    c
}

pub fn hash_uint2(kx: u32, ky: u32) -> u32 {
    let mut a = 0xdeadbeefu32.wrapping_add(2 << 2).wrapping_add(13);
    let (mut b, mut c) = (a, a);
    b = b.wrapping_add(ky);
    a = a.wrapping_add(kx);
    final_mix(&mut a, &mut b, &mut c);
    c
}

pub fn hash_uint3(kx: u32, ky: u32, kz: u32) -> u32 {
    let mut a = 0xdeadbeefu32.wrapping_add(3 << 2).wrapping_add(13);
    let (mut b, mut c) = (a, a);
    c = c.wrapping_add(kz);
    b = b.wrapping_add(ky);
    a = a.wrapping_add(kx);
    final_mix(&mut a, &mut b, &mut c);
    c
}

pub fn hash_uint4(kx: u32, ky: u32, kz: u32, kw: u32) -> u32 {
    let mut a = 0xdeadbeefu32.wrapping_add(4 << 2).wrapping_add(13);
    let (mut b, mut c) = (a, a);
    a = a.wrapping_add(kx);
    b = b.wrapping_add(ky);
    c = c.wrapping_add(kz);
    mix(&mut a, &mut b, &mut c);
    a = a.wrapping_add(kw);
    final_mix(&mut a, &mut b, &mut c);
    c
}

const UINT_MAX_F: f32 = 4294967295.0;

pub fn hash_f1(k: f32) -> f32 {
    hash_uint(k.to_bits()) as f32 / UINT_MAX_F
}
pub fn hash_f2(k: [f32; 2]) -> f32 {
    hash_uint2(k[0].to_bits(), k[1].to_bits()) as f32 / UINT_MAX_F
}
pub fn hash_f3(k: [f32; 3]) -> f32 {
    hash_uint3(k[0].to_bits(), k[1].to_bits(), k[2].to_bits()) as f32 / UINT_MAX_F
}
pub fn hash_f4(k: [f32; 4]) -> f32 {
    hash_uint4(k[0].to_bits(), k[1].to_bits(), k[2].to_bits(), k[3].to_bits()) as f32 / UINT_MAX_F
}

/// Hash of an N-D point to one float in [0, 1].
pub fn hash_n<const N: usize>(k: [f32; N]) -> f32 {
    match N {
        1 => hash_f1(k[0]),
        2 => hash_f2([k[0], k[1]]),
        3 => hash_f3([k[0], k[1], k[2]]),
        _ => hash_f4([k[0], k[1], k[2], k[3]]),
    }
}

/// `hash_floatN_to_float3`: a colour from an N-D point.
pub fn hash_to_color<const N: usize>(k: [f32; N]) -> [f32; 3] {
    match N {
        1 => [hash_f1(k[0]), hash_f2([k[0], 1.0]), hash_f2([k[0], 2.0])],
        2 => [hash_f2([k[0], k[1]]), hash_f3([k[0], k[1], 1.0]), hash_f3([k[0], k[1], 2.0])],
        3 => [hash_f3([k[0], k[1], k[2]]), hash_f4([k[0], k[1], k[2], 1.0]), hash_f4([k[0], k[1], k[2], 2.0])],
        _ => {
            let k = [k[0], k[1], k[2], k[3]];
            [hash_f4(k), hash_f4([k[2], k[0], k[3], k[1]]), hash_f4([k[3], k[2], k[1], k[0]])]
        }
    }
}

/// `hash_floatN_to_floatN`: Voronoi feature point offsets.
pub fn hash_to_vec<const N: usize>(k: [f32; N]) -> [f32; N] {
    let mut o = [0.0; N];
    match N {
        1 => o[0] = hash_f1(k[0]),
        2 => {
            o[0] = hash_f2([k[0], k[1]]);
            o[1] = hash_f3([k[0], k[1], 1.0]);
        }
        3 => {
            let c = hash_to_color(k);
            o[..3].copy_from_slice(&c);
        }
        _ => {
            let k = [k[0], k[1], k[2], k[3]];
            o[0] = hash_f4(k);
            o[1] = hash_f4([k[3], k[0], k[1], k[2]]);
            o[2] = hash_f4([k[2], k[3], k[0], k[1]]);
            o[3] = hash_f4([k[1], k[2], k[3], k[0]]);
        }
    }
    o
}

// ---------------------------------------------------------------------------
// Perlin noise (`noise.h`)

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn negate_if(v: f32, c: u32) -> f32 {
    if c != 0 {
        -v
    } else {
        v
    }
}

#[inline]
fn floorfrac(x: f32) -> (i32, f32) {
    let i = x.floor() as i32;
    (i, x - i as f32)
}

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (1.0 - t) * a + t * b
}

#[inline]
fn bi_mix(v0: f32, v1: f32, v2: f32, v3: f32, x: f32, y: f32) -> f32 {
    let x1 = 1.0 - x;
    (1.0 - y) * (v0 * x1 + v1 * x) + y * (v2 * x1 + v3 * x)
}

fn grad1(hash: u32, x: f32) -> f32 {
    let h = hash & 15;
    let g = 1.0 + (h & 7) as f32;
    negate_if(g, h & 8) * x
}

fn grad2(hash: u32, x: f32, y: f32) -> f32 {
    let h = hash & 7;
    let u = if h < 4 { x } else { y };
    let v = 2.0 * if h < 4 { y } else { x };
    negate_if(u, h & 1) + negate_if(v, h & 2)
}

fn grad3(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let vt = if h == 12 || h == 14 { x } else { z };
    let v = if h < 4 { y } else { vt };
    negate_if(u, h & 1) + negate_if(v, h & 2)
}

fn grad4(hash: u32, x: f32, y: f32, z: f32, w: f32) -> f32 {
    let h = hash & 31;
    let u = if h < 24 { x } else { y };
    let v = if h < 16 { y } else { z };
    let s = if h < 8 { z } else { w };
    negate_if(u, h & 1) + negate_if(v, h & 2) + negate_if(s, h & 4)
}

fn perlin_1d(x: f32) -> f32 {
    let (xi, fx) = floorfrac(x);
    let u = fade(fx);
    lerp(grad1(hash_uint(xi as u32), fx), grad1(hash_uint(xi.wrapping_add(1) as u32), fx - 1.0), u)
}

fn perlin_2d(x: f32, y: f32) -> f32 {
    let ((xi, fx), (yi, fy)) = (floorfrac(x), floorfrac(y));
    let (u, v) = (fade(fx), fade(fy));
    let h = |dx: i32, dy: i32| hash_uint2(xi.wrapping_add(dx) as u32, yi.wrapping_add(dy) as u32);
    bi_mix(grad2(h(0, 0), fx, fy), grad2(h(1, 0), fx - 1.0, fy), grad2(h(0, 1), fx, fy - 1.0), grad2(h(1, 1), fx - 1.0, fy - 1.0), u, v)
}

fn perlin_3d(x: f32, y: f32, z: f32) -> f32 {
    let ((xi, fx), (yi, fy), (zi, fz)) = (floorfrac(x), floorfrac(y), floorfrac(z));
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let g = |dx: i32, dy: i32, dz: i32| {
        let hh = hash_uint3(xi.wrapping_add(dx) as u32, yi.wrapping_add(dy) as u32, zi.wrapping_add(dz) as u32);
        grad3(hh, fx - dx as f32, fy - dy as f32, fz - dz as f32)
    };
    let a = bi_mix(g(0, 0, 0), g(1, 0, 0), g(0, 1, 0), g(1, 1, 0), u, v);
    let b = bi_mix(g(0, 0, 1), g(1, 0, 1), g(0, 1, 1), g(1, 1, 1), u, v);
    lerp(a, b, w)
}

fn perlin_4d(x: f32, y: f32, z: f32, w: f32) -> f32 {
    let ((xi, fx), (yi, fy), (zi, fz), (wi, fw)) = (floorfrac(x), floorfrac(y), floorfrac(z), floorfrac(w));
    let (u, v, t, s) = (fade(fx), fade(fy), fade(fz), fade(fw));
    let g = |dx: i32, dy: i32, dz: i32, dw: i32| {
        let hh = hash_uint4(xi.wrapping_add(dx) as u32, yi.wrapping_add(dy) as u32, zi.wrapping_add(dz) as u32, wi.wrapping_add(dw) as u32);
        grad4(hh, fx - dx as f32, fy - dy as f32, fz - dz as f32, fw - dw as f32)
    };
    let cube = |dw: i32| {
        let a = bi_mix(g(0, 0, 0, dw), g(1, 0, 0, dw), g(0, 1, 0, dw), g(1, 1, 0, dw), u, v);
        let b = bi_mix(g(0, 0, 1, dw), g(1, 0, 1, dw), g(0, 1, 1, dw), g(1, 1, 1, dw), u, v);
        lerp(a, b, t)
    };
    lerp(cube(0), cube(1), s)
}

/// Large inputs lose precision in floats: wrap them like Cycles does.
#[inline]
fn precision_wrap(p: f32) -> f32 {
    let corr = if p.abs() >= 1_000_000.0 { 0.5 } else { 0.0 };
    p % 100_000.0 + corr
}

fn finite_or_zero(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

/// Signed Perlin noise in about [-1, 1] (`snoise_Nd`).
pub fn snoise<const N: usize>(p: [f32; N]) -> f32 {
    let q: [f32; N] = p.map(precision_wrap);
    match N {
        1 => 0.2500 * finite_or_zero(perlin_1d(q[0])),
        2 => 0.6616 * finite_or_zero(perlin_2d(q[0], q[1])),
        3 => 0.9820 * finite_or_zero(perlin_3d(q[0], q[1], q[2])),
        _ => 0.8344 * finite_or_zero(perlin_4d(q[0], q[1], q[2], q[3])),
    }
}

fn scale<const N: usize>(p: [f32; N], s: f32) -> [f32; N] {
    p.map(|x| x * s)
}

fn add<const N: usize>(a: [f32; N], b: [f32; N]) -> [f32; N] {
    let mut o = a;
    for i in 0..N {
        o[i] += b[i];
    }
    o
}

/// The fractal Noise Texture types (Blender 4.1+ merged Musgrave into Noise).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseType {
    MultiFractal,
    RidgedMultiFractal,
    HybridMultiFractal,
    Fbm,
    HeteroTerrain,
}

impl NoiseType {
    pub fn from_name(s: &str) -> NoiseType {
        match s {
            "MULTIFRACTAL" => NoiseType::MultiFractal,
            "RIDGED_MULTIFRACTAL" => NoiseType::RidgedMultiFractal,
            "HYBRID_MULTIFRACTAL" => NoiseType::HybridMultiFractal,
            "HETERO_TERRAIN" => NoiseType::HeteroTerrain,
            _ => NoiseType::Fbm,
        }
    }
}

/// Fractal parameters shared by the noise types.
#[derive(Clone, Copy, Debug)]
pub struct Fractal {
    pub detail: f32,
    pub roughness: f32,
    pub lacunarity: f32,
    pub offset: f32,
    pub gain: f32,
    pub normalize: bool,
}

pub fn noise_fbm<const N: usize>(p: [f32; N], detail: f32, roughness: f32, lacunarity: f32, normalize: bool) -> f32 {
    let mut fscale = 1.0;
    let mut amp = 1.0;
    let mut maxamp = 0.0;
    let mut sum = 0.0;
    for _ in 0..=(detail as i32) {
        let t = snoise(scale(p, fscale));
        sum += t * amp;
        maxamp += amp;
        amp *= roughness;
        fscale *= lacunarity;
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 {
        let t = snoise(scale(p, fscale));
        let sum2 = sum + t * amp;
        if normalize {
            lerp(0.5 * sum / maxamp + 0.5, 0.5 * sum2 / (maxamp + amp) + 0.5, rmd)
        } else {
            lerp(sum, sum2, rmd)
        }
    } else if normalize {
        0.5 * sum / maxamp + 0.5
    } else {
        sum
    }
}

fn noise_multi_fractal<const N: usize>(mut p: [f32; N], detail: f32, roughness: f32, lacunarity: f32) -> f32 {
    let mut value = 1.0;
    let mut pwr = 1.0;
    for _ in 0..=(detail as i32) {
        value *= pwr * snoise(p) + 1.0;
        pwr *= roughness;
        p = scale(p, lacunarity);
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 {
        value *= rmd * pwr * snoise(p) + 1.0;
    }
    value
}

fn noise_hetero_terrain<const N: usize>(mut p: [f32; N], detail: f32, roughness: f32, lacunarity: f32, offset: f32) -> f32 {
    let mut pwr = roughness;
    let mut value = offset + snoise(p);
    p = scale(p, lacunarity);
    for _ in 1..=(detail as i32) {
        let increment = (snoise(p) + offset) * pwr * value;
        value += increment;
        pwr *= roughness;
        p = scale(p, lacunarity);
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 {
        let increment = (snoise(p) + offset) * pwr * value;
        value += rmd * increment;
    }
    value
}

fn noise_hybrid_multi_fractal<const N: usize>(mut p: [f32; N], detail: f32, roughness: f32, lacunarity: f32, offset: f32, gain: f32) -> f32 {
    let mut pwr = 1.0;
    let mut value = 0.0;
    let mut weight = 1.0f32;
    let mut i = 0;
    while weight > 0.001 && i <= detail as i32 {
        if weight > 1.0 {
            weight = 1.0;
        }
        let signal = (snoise(p) + offset) * pwr;
        pwr *= roughness;
        value += weight * signal;
        weight *= gain * signal;
        p = scale(p, lacunarity);
        i += 1;
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 && weight > 0.001 {
        if weight > 1.0 {
            weight = 1.0;
        }
        let signal = (snoise(p) + offset) * pwr;
        value += rmd * weight * signal;
    }
    value
}

fn noise_ridged_multi_fractal<const N: usize>(mut p: [f32; N], detail: f32, roughness: f32, lacunarity: f32, offset: f32, gain: f32) -> f32 {
    let mut pwr = roughness;
    let mut signal = offset - snoise(p).abs();
    signal *= signal;
    let mut value = signal;
    for _ in 1..=(detail as i32) {
        p = scale(p, lacunarity);
        let weight = (signal * gain).clamp(0.0, 1.0);
        signal = offset - snoise(p).abs();
        signal *= signal;
        signal *= weight;
        value += signal * pwr;
        pwr *= roughness;
    }
    value
}

pub fn noise_select<const N: usize>(p: [f32; N], f: &Fractal, ty: NoiseType) -> f32 {
    match ty {
        NoiseType::MultiFractal => noise_multi_fractal(p, f.detail, f.roughness, f.lacunarity),
        NoiseType::Fbm => noise_fbm(p, f.detail, f.roughness, f.lacunarity, f.normalize),
        NoiseType::HybridMultiFractal => noise_hybrid_multi_fractal(p, f.detail, f.roughness, f.lacunarity, f.offset, f.gain),
        NoiseType::RidgedMultiFractal => noise_ridged_multi_fractal(p, f.detail, f.roughness, f.lacunarity, f.offset, f.gain),
        NoiseType::HeteroTerrain => noise_hetero_terrain(p, f.detail, f.roughness, f.lacunarity, f.offset),
    }
}

/// `random_floatN_offset(seed)`: decorrelating offsets for the colour channels.
fn random_offset<const N: usize>(seed: f32) -> [f32; N] {
    let mut o = [0.0; N];
    if N == 1 {
        o[0] = 100.0 + hash_f1(seed) * 100.0;
    } else {
        for (i, x) in o.iter_mut().enumerate() {
            *x = 100.0 + hash_f2([seed, i as f32]) * 100.0;
        }
    }
    o
}

/// Noise Texture: (Fac, Color) at the already scaled point `p`.
pub fn noise_texture<const N: usize>(p: [f32; N], f: &Fractal, ty: NoiseType, distortion: f32, want_color: bool) -> (f32, [f32; 3]) {
    let mut p = p;
    if distortion != 0.0 {
        let mut d = [0.0; N];
        for (i, x) in d.iter_mut().enumerate() {
            *x = snoise(add(p, random_offset::<N>(i as f32))) * distortion;
        }
        p = add(p, d);
    }
    let v = noise_select(p, f, ty);
    let c = if want_color { [v, noise_select(add(p, random_offset::<N>(N as f32)), f, ty), noise_select(add(p, random_offset::<N>(N as f32 + 1.0)), f, ty)] } else { [v; 3] };
    (v, c)
}

// ---------------------------------------------------------------------------
// Voronoi (`voronoi.h`, Blender 4.x with fractal detail)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoronoiFeature {
    F1,
    F2,
    SmoothF1,
    DistanceToEdge,
    NSphereRadius,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoronoiMetric {
    Euclidean,
    Manhattan,
    Chebychev,
    Minkowski,
}

#[derive(Clone, Copy, Debug)]
pub struct VoronoiParams {
    pub scale: f32,
    pub detail: f32,
    pub roughness: f32,
    pub lacunarity: f32,
    /// Already halved and clamped to [0, 0.5], as Cycles does.
    pub smoothness: f32,
    pub exponent: f32,
    pub randomness: f32,
    pub max_distance: f32,
    pub normalize: bool,
    pub feature: VoronoiFeature,
    pub metric: VoronoiMetric,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VoronoiOutput {
    pub distance: f32,
    pub color: [f32; 3],
    /// Position in up to four dimensions (x, y, z, w; 1D keeps it in x).
    pub position: [f32; 4],
}

fn vdist<const N: usize>(a: [f32; N], b: [f32; N], p: &VoronoiParams) -> f32 {
    if N == 1 {
        return (a[0] - b[0]).abs();
    }
    let d = |i: usize| (a[i] - b[i]).abs();
    match p.metric {
        VoronoiMetric::Euclidean => (0..N).map(|i| d(i) * d(i)).sum::<f32>().sqrt(),
        VoronoiMetric::Manhattan => (0..N).map(d).sum(),
        VoronoiMetric::Chebychev => (0..N).map(d).fold(0.0, f32::max),
        VoronoiMetric::Minkowski => (0..N).map(|i| d(i).powf(p.exponent)).sum::<f32>().powf(1.0 / p.exponent),
    }
}

/// Neighbour cell offsets in Cycles' loop order (x fastest), radius `r`.
fn offsets<const N: usize>(r: i32) -> impl Iterator<Item = [f32; N]> {
    let w = 2 * r + 1;
    (0..w.pow(N as u32)).map(move |m| {
        let mut o = [0.0; N];
        let mut k = m;
        for x in o.iter_mut() {
            *x = (k % w - r) as f32;
            k /= w;
        }
        o
    })
}

fn to4<const N: usize>(a: [f32; N]) -> [f32; 4] {
    let mut o = [0.0; 4];
    o[..N].copy_from_slice(&a);
    o
}

fn split<const N: usize>(coord: [f32; N]) -> ([f32; N], [f32; N]) {
    let cell = coord.map(f32::floor);
    let mut local = coord;
    for i in 0..N {
        local[i] -= cell[i];
    }
    (cell, local)
}

fn feature_point<const N: usize>(cell: [f32; N], off: [f32; N], randomness: f32) -> [f32; N] {
    let h = hash_to_vec(add(cell, off));
    let mut p = off;
    for i in 0..N {
        p[i] += h[i] * randomness;
    }
    p
}

fn voronoi_f1<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> VoronoiOutput {
    let (cell, local) = split(coord);
    let mut min_d = f32::MAX;
    let mut target_off = [0.0; N];
    let mut target_pos = [0.0; N];
    for off in offsets::<N>(1) {
        let pp = feature_point(cell, off, p.randomness);
        let d = vdist(pp, local, p);
        if d < min_d {
            target_off = off;
            min_d = d;
            target_pos = pp;
        }
    }
    VoronoiOutput { distance: min_d, color: hash_to_color(add(cell, target_off)), position: to4(add(target_pos, cell)) }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    if x < e0 {
        0.0
    } else if x >= e1 {
        1.0
    } else {
        let t = (x - e0) / (e1 - e0);
        (3.0 - 2.0 * t) * t * t
    }
}

fn voronoi_smooth_f1<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> VoronoiOutput {
    let (cell, local) = split(coord);
    let mut sd = 0.0;
    let mut sc = [0.0f32; 3];
    let mut sp = [0.0f32; N];
    let mut h = -1.0f32;
    for off in offsets::<N>(2) {
        let pp = feature_point(cell, off, p.randomness);
        let d = vdist(pp, local, p);
        h = if h == -1.0 { 1.0 } else { smoothstep(0.0, 1.0, 0.5 + 0.5 * (sd - d) / p.smoothness) };
        let mut corr = p.smoothness * h * (1.0 - h);
        sd = lerp(sd, d, h) - corr;
        corr /= 1.0 + 3.0 * p.smoothness;
        let cc = hash_to_color(add(cell, off));
        for i in 0..3 {
            sc[i] = lerp(sc[i], cc[i], h) - corr;
        }
        for i in 0..N {
            sp[i] = lerp(sp[i], pp[i], h) - corr;
        }
    }
    VoronoiOutput { distance: sd, color: sc, position: to4(add(cell, sp)) }
}

fn voronoi_f2<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> VoronoiOutput {
    let (cell, local) = split(coord);
    let (mut d1, mut d2) = (f32::MAX, f32::MAX);
    let (mut o1, mut o2) = ([0.0; N], [0.0; N]);
    let (mut p1, mut p2) = ([0.0; N], [0.0; N]);
    for off in offsets::<N>(1) {
        let pp = feature_point(cell, off, p.randomness);
        let d = vdist(pp, local, p);
        if d < d1 {
            d2 = d1;
            d1 = d;
            o2 = o1;
            o1 = off;
            p2 = p1;
            p1 = pp;
        } else if d < d2 {
            d2 = d;
            o2 = off;
            p2 = pp;
        }
    }
    VoronoiOutput { distance: d2, color: hash_to_color(add(cell, o2)), position: to4(add(p2, cell)) }
}

fn voronoi_distance_to_edge<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> f32 {
    let (cell, local) = split(coord);
    if N == 1 {
        let c = cell[0];
        let l = local[0];
        let mid = hash_f1(c) * p.randomness;
        let left = -1.0 + hash_f1(c - 1.0) * p.randomness;
        let right = 1.0 + hash_f1(c + 1.0) * p.randomness;
        return ((mid + left) / 2.0 - l).abs().min(((mid + right) / 2.0 - l).abs());
    }
    let dot = |a: [f32; N], b: [f32; N]| (0..N).map(|i| a[i] * b[i]).sum::<f32>();
    let to_point = |off: [f32; N]| {
        let mut v = feature_point(cell, off, p.randomness);
        for i in 0..N {
            v[i] -= local[i];
        }
        v
    };
    let mut closest = [0.0; N];
    let mut min_d = f32::MAX;
    for off in offsets::<N>(1) {
        let v = to_point(off);
        let d = dot(v, v);
        if d < min_d {
            min_d = d;
            closest = v;
        }
    }
    min_d = f32::MAX;
    for off in offsets::<N>(1) {
        let v = to_point(off);
        let mut perp = v;
        for i in 0..N {
            perp[i] -= closest[i];
        }
        let pl = dot(perp, perp);
        if pl > 0.0001 {
            let inv = 1.0 / pl.sqrt();
            let mut mid = [0.0; N];
            for i in 0..N {
                mid[i] = (closest[i] + v[i]) / 2.0;
            }
            let de = (0..N).map(|i| mid[i] * perp[i] * inv).sum::<f32>();
            min_d = min_d.min(de);
        }
    }
    min_d
}

fn voronoi_n_sphere_radius<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> f32 {
    let (cell, local) = split(coord);
    let mut closest = [0.0; N];
    let mut closest_off = [0.0; N];
    let mut min_d = f32::MAX;
    let euclid = |a: [f32; N], b: [f32; N]| (0..N).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum::<f32>().sqrt();
    for off in offsets::<N>(1) {
        let pp = feature_point(cell, off, p.randomness);
        let d = euclid(pp, local);
        if d < min_d {
            min_d = d;
            closest = pp;
            closest_off = off;
        }
    }
    min_d = f32::MAX;
    let mut cc = [0.0; N];
    for off in offsets::<N>(1) {
        if off.iter().all(|&x| x == 0.0) {
            continue;
        }
        let o = add(off, closest_off);
        let pp = feature_point(cell, o, p.randomness);
        let d = euclid(closest, pp);
        if d < min_d {
            min_d = d;
            cc = pp;
        }
    }
    euclid(cc, closest) / 2.0
}

fn octave<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> VoronoiOutput {
    match p.feature {
        VoronoiFeature::F2 => voronoi_f2(p, coord),
        VoronoiFeature::SmoothF1 if p.smoothness != 0.0 => voronoi_smooth_f1(p, coord),
        _ => voronoi_f1(p, coord),
    }
}

/// `fractal_voronoi_x_fx`: F1/F2/Smooth F1 with fractal detail.
fn fractal_voronoi<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> VoronoiOutput {
    let mut amplitude = 1.0;
    let mut max_amplitude = 0.0;
    let mut sc = 1.0;
    let mut out = VoronoiOutput::default();
    let zero_input = p.detail == 0.0 || p.roughness == 0.0;
    let mut i = 0;
    while i as f32 <= p.detail.ceil() {
        let o = octave(p, scale(coord, sc));
        if zero_input {
            max_amplitude = 1.0;
            out = o;
            break;
        } else if i as f32 <= p.detail {
            max_amplitude += amplitude;
            out.distance += o.distance * amplitude;
            for k in 0..3 {
                out.color[k] += o.color[k] * amplitude;
            }
            for k in 0..4 {
                out.position[k] = lerp(out.position[k], o.position[k] / sc, amplitude);
            }
            sc *= p.lacunarity;
            amplitude *= p.roughness;
        } else {
            let rem = p.detail - p.detail.floor();
            if rem != 0.0 {
                max_amplitude = lerp(max_amplitude, max_amplitude + amplitude, rem);
                out.distance = lerp(out.distance, out.distance + o.distance * amplitude, rem);
                for k in 0..3 {
                    out.color[k] = lerp(out.color[k], out.color[k] + o.color[k] * amplitude, rem);
                }
                for k in 0..4 {
                    out.position[k] = lerp(out.position[k], lerp(out.position[k], o.position[k] / sc, amplitude), rem);
                }
            }
        }
        i += 1;
    }
    if p.normalize {
        out.distance /= max_amplitude * p.max_distance;
        for c in &mut out.color {
            *c /= max_amplitude;
        }
    }
    for x in &mut out.position {
        *x = if p.scale != 0.0 { *x / p.scale } else { 0.0 };
    }
    out
}

fn fractal_voronoi_distance_to_edge<const N: usize>(p: &VoronoiParams, coord: [f32; N]) -> f32 {
    let mut amplitude = 1.0;
    let mut max_amplitude = p.max_distance;
    let mut sc = 1.0;
    let mut distance = 8.0f32;
    let zero_input = p.detail == 0.0 || p.roughness == 0.0;
    let mut i = 0;
    while i as f32 <= p.detail.ceil() {
        let od = voronoi_distance_to_edge(p, scale(coord, sc));
        if zero_input {
            distance = od;
            break;
        } else if i as f32 <= p.detail {
            max_amplitude = lerp(max_amplitude, p.max_distance / sc, amplitude);
            distance = lerp(distance, distance.min(od / sc), amplitude);
            sc *= p.lacunarity;
            amplitude *= p.roughness;
        } else {
            let rem = p.detail - p.detail.floor();
            if rem != 0.0 {
                let la = lerp(max_amplitude, p.max_distance / sc, amplitude);
                max_amplitude = lerp(max_amplitude, la, rem);
                let ld = lerp(distance, distance.min(od / sc), amplitude);
                distance = lerp(distance, distance.min(ld), rem);
            }
        }
        i += 1;
    }
    if p.normalize {
        distance /= max_amplitude;
    }
    distance
}

/// Voronoi Texture at the already scaled point; `params.max_distance` is
/// filled in here.
pub fn voronoi<const N: usize>(mut params: VoronoiParams, coord: [f32; N]) -> (VoronoiOutput, f32) {
    let r = 0.5 + 0.5 * params.randomness;
    params.max_distance = match params.feature {
        VoronoiFeature::DistanceToEdge => r,
        _ => {
            let d = if N == 1 { r } else { vdist([0.0; N], [r; N], &params) };
            d * if params.feature == VoronoiFeature::F2 { 2.0 } else { 1.0 }
        }
    };
    match params.feature {
        VoronoiFeature::DistanceToEdge => (VoronoiOutput { distance: fractal_voronoi_distance_to_edge(&params, coord), ..Default::default() }, 0.0),
        VoronoiFeature::NSphereRadius => (VoronoiOutput::default(), voronoi_n_sphere_radius(&params, coord)),
        _ => (fractal_voronoi(&params, coord), 0.0),
    }
}

// ---------------------------------------------------------------------------
// Other procedurals

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveType {
    Bands,
    Rings,
}

/// Wave Texture (`svm_wave`): direction 0..3 = X, Y, Z, diagonal/spherical;
/// profile 0 sine, 1 saw, 2 triangle. `p` is already scaled.
#[allow(clippy::too_many_arguments)]
pub fn wave(ty: WaveType, direction: u8, profile: u8, p: [f32; 3], distortion: f32, detail: f32, dscale: f32, droughness: f32, phase: f32) -> f32 {
    let p = p.map(|x| (x + 0.000001) * 0.999999);
    let mut n = match ty {
        WaveType::Bands => match direction {
            0 => p[0] * 20.0,
            1 => p[1] * 20.0,
            2 => p[2] * 20.0,
            _ => (p[0] + p[1] + p[2]) * 10.0,
        },
        WaveType::Rings => {
            let mut rp = p;
            match direction {
                0 => rp[0] = 0.0,
                1 => rp[1] = 0.0,
                2 => rp[2] = 0.0,
                _ => {}
            }
            (rp[0] * rp[0] + rp[1] * rp[1] + rp[2] * rp[2]).sqrt() * 20.0
        }
    };
    n += phase;
    if distortion != 0.0 {
        n += distortion * (noise_fbm(p.map(|x| x * dscale), detail, droughness, 2.0, true) * 2.0 - 1.0);
    }
    match profile {
        0 => 0.5 + 0.5 * (n - FRAC_PI_2).sin(),
        1 => {
            let n = n / TAU;
            n - n.floor()
        }
        _ => {
            let n = n / TAU;
            (n - (n + 0.5).floor()).abs() * 2.0
        }
    }
}

/// Magic Texture (`svm_magic`) at the already scaled point.
pub fn magic(p: [f32; 3], depth: i32, distortion: f32) -> [f32; 3] {
    let mut x = ((p[0] + p[1] + p[2]) * 5.0).sin();
    let mut y = ((-p[0] + p[1] - p[2]) * 5.0).cos();
    let mut z = -((-p[0] - p[1] + p[2]) * 5.0).cos();
    let d = distortion;
    if depth > 0 {
        x *= d;
        y *= d;
        z *= d;
        y = -(x - y + z).cos();
        y *= d;
        if depth > 1 {
            x = (x - y - z).cos();
            x *= d;
            if depth > 2 {
                z = (-x - y - z).sin();
                z *= d;
                if depth > 3 {
                    x = -(-x + y - z).cos();
                    x *= d;
                    if depth > 4 {
                        y = -(-x + y + z).sin();
                        y *= d;
                        if depth > 5 {
                            y = -(-x + y + z).cos();
                            y *= d;
                            if depth > 6 {
                                x = (x + y + z).cos();
                                x *= d;
                                if depth > 7 {
                                    z = (x + y - z).sin();
                                    z *= d;
                                    if depth > 8 {
                                        x = -(-x - y + z).cos();
                                        x *= d;
                                        if depth > 9 {
                                            y = -(x - y + z).sin();
                                            y *= d;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if d != 0.0 {
        let d2 = d * 2.0;
        x /= d2;
        y /= d2;
        z /= d2;
    }
    [0.5 - x, 0.5 - y, 0.5 - z]
}

/// Checker Texture (`svm_checker`) at the already scaled point: 1 or 0.
pub fn checker(p: [f32; 3]) -> f32 {
    let p = p.map(|x| (x + 0.000001) * 0.999999);
    let xi = (p[0].floor() as i32).abs();
    let yi = (p[1].floor() as i32).abs();
    let zi = (p[2].floor() as i32).abs();
    if ((xi % 2 == yi % 2) as i32) == zi % 2 {
        1.0
    } else {
        0.0
    }
}

fn brick_noise(n: u32) -> f32 {
    let n = n.wrapping_add(1013) & 0x7fff_ffff;
    let n = (n >> 13) ^ n;
    let nn = n.wrapping_mul(n.wrapping_mul(n).wrapping_mul(60493).wrapping_add(19990303)).wrapping_add(1376312589) & 0x7fff_ffff;
    0.5 * (nn as f32 / 1073741824.0)
}

/// Brick Texture settings (the node's properties and inputs).
#[derive(Clone, Copy, Debug)]
pub struct Brick {
    pub mortar_size: f32,
    pub mortar_smooth: f32,
    pub bias: f32,
    pub brick_width: f32,
    pub row_height: f32,
    pub offset: f32,
    pub offset_frequency: i32,
    pub squash: f32,
    pub squash_frequency: i32,
}

/// `svm_brick`: (tint, mortar factor) at the already scaled point.
pub fn brick(p: [f32; 3], b: &Brick) -> (f32, f32) {
    let mut brick_width = b.brick_width;
    let row_height = b.row_height;
    let mut offset = 0.0;
    let rownum = (p[1] / row_height).floor() as i32;
    if b.offset_frequency != 0 && b.squash_frequency != 0 {
        brick_width *= if rownum.wrapping_rem(b.squash_frequency) != 0 { 1.0 } else { b.squash };
        offset = if rownum.wrapping_rem(b.offset_frequency) != 0 { 0.0 } else { brick_width * b.offset };
    }
    let bricknum = ((p[0] + offset) / brick_width).floor() as i32;
    let x = (p[0] + offset) - brick_width * bricknum as f32;
    let y = p[1] - row_height * rownum as f32;
    let seed = (rownum.wrapping_shl(16)).wrapping_add(bricknum & 0xFFFF) as u32;
    let tint = (brick_noise(seed) + b.bias).clamp(0.0, 1.0);
    let min_dist = x.min(y).min((brick_width - x).min(row_height - y));
    let mortar = if min_dist >= b.mortar_size {
        0.0
    } else if b.mortar_smooth == 0.0 {
        1.0
    } else {
        let md = 1.0 - min_dist / b.mortar_size;
        if md < b.mortar_smooth {
            let f = md / b.mortar_smooth;
            let ff = f * f;
            3.0 * ff - 2.0 * ff * f
        } else {
            1.0
        }
    };
    (tint, mortar)
}

/// Gradient Texture types in Blender's enum order.
pub fn gradient(p: [f32; 3], ty: u8) -> f32 {
    let (x, y, z) = (p[0], p[1], p[2]);
    let f = match ty {
        0 => x,
        1 => {
            let r = x.max(0.0);
            r * r
        }
        2 => {
            let r = x.clamp(0.0, 1.0);
            let t = r * r;
            3.0 * t - 2.0 * t * r
        }
        3 => (x + y) * 0.5,
        4 => y.atan2(x) / TAU + 0.5,
        _ => {
            let r = (1.0 - (x * x + y * y + z * z).sqrt()).max(0.0);
            if ty == 5 {
                r * r
            } else {
                r
            }
        }
    };
    f.clamp(0.0, 1.0)
}

/// White Noise Texture: (Value, Color).
pub fn white_noise<const N: usize>(p: [f32; N]) -> (f32, [f32; 3]) {
    (hash_n(p), hash_to_color(p))
}

// ---------------------------------------------------------------------------
// Gabor noise (Blender 4.3's `gabor.h`)

const GABOR_IMPULSES: usize = 8;

fn hash_f3_to_f2(k: [f32; 3]) -> [f32; 2] {
    [hash_f3(k), hash_f4([k[0], k[1], k[2], 1.0])]
}

fn hash_f4_to_f2(k: [f32; 4]) -> [f32; 2] {
    [hash_f4(k), hash_f4([k[3], k[0], k[1], k[2]])]
}

fn hash_f4_to_f3(k: [f32; 4]) -> [f32; 3] {
    hash_to_color(k)
}

fn gabor_kernel_2d(p: [f32; 2], frequency: f32, orientation: f32) -> [f32; 2] {
    let d2 = p[0] * p[0] + p[1] * p[1];
    let hann = 0.5 + 0.5 * (PI * d2).cos();
    let env = (-PI * d2).exp() * hann;
    let fv = [frequency * orientation.cos(), frequency * orientation.sin()];
    let angle = TAU * (p[0] * fv[0] + p[1] * fv[1]);
    [env * angle.cos(), env * angle.sin()]
}

fn gabor_kernel_3d(p: [f32; 3], frequency: f32, orientation: [f32; 3]) -> [f32; 2] {
    let d2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
    let hann = 0.5 + 0.5 * (PI * d2).cos();
    let env = (-PI * d2).exp() * hann;
    let angle = TAU * frequency * (p[0] * orientation[0] + p[1] * orientation[1] + p[2] * orientation[2]);
    [env * angle.cos(), env * angle.sin()]
}

fn gabor_cell_2d(cell: [f32; 2], pos: [f32; 2], frequency: f32, isotropy: f32, base: f32) -> [f32; 2] {
    let mut noise = [0.0; 2];
    for i in 0..GABOR_IMPULSES {
        let fi = i as f32 * 3.0;
        let rand_orient = (hash_f3([cell[0], cell[1], fi]) - 0.5) * PI;
        let orientation = base + rand_orient * isotropy;
        let center = hash_f3_to_f2([cell[0], cell[1], fi + 1.0]);
        let pk = [pos[0] - center[0], pos[1] - center[1]];
        if pk[0] * pk[0] + pk[1] * pk[1] >= 1.0 {
            continue;
        }
        let w = if hash_f3([cell[0], cell[1], fi + 2.0]) < 0.5 { -1.0 } else { 1.0 };
        let k = gabor_kernel_2d(pk, frequency, orientation);
        noise[0] += w * k[0];
        noise[1] += w * k[1];
    }
    noise
}

fn gabor_orientation_3d(orientation: [f32; 3], isotropy: f32, seed: [f32; 4]) -> [f32; 3] {
    if isotropy == 0.0 {
        return orientation;
    }
    let mut inclination = orientation[2].clamp(-1.0, 1.0).acos();
    let rxy = (orientation[0] * orientation[0] + orientation[1] * orientation[1]).sqrt();
    let sgn = if orientation[1] > 0.0 {
        1.0
    } else if orientation[1] < 0.0 {
        -1.0
    } else {
        0.0
    };
    let mut azimuth = sgn * if rxy > 0.0 { (orientation[0] / rxy).clamp(-1.0, 1.0).acos() } else { 0.0 };
    let r = hash_f4_to_f2(seed);
    inclination += r[0] * PI * isotropy;
    azimuth += r[1] * PI * isotropy;
    [inclination.sin() * azimuth.cos(), inclination.sin() * azimuth.sin(), inclination.cos()]
}

fn gabor_cell_3d(cell: [f32; 3], pos: [f32; 3], frequency: f32, isotropy: f32, base: [f32; 3]) -> [f32; 2] {
    let mut noise = [0.0; 2];
    for i in 0..GABOR_IMPULSES {
        let fi = i as f32 * 3.0;
        let orientation = gabor_orientation_3d(base, isotropy, [cell[0], cell[1], cell[2], fi]);
        let center = hash_f4_to_f3([cell[0], cell[1], cell[2], fi + 1.0]);
        let pk = [pos[0] - center[0], pos[1] - center[1], pos[2] - center[2]];
        if pk[0] * pk[0] + pk[1] * pk[1] + pk[2] * pk[2] >= 1.0 {
            continue;
        }
        let w = if hash_f4([cell[0], cell[1], cell[2], fi + 2.0]) < 0.5 { -1.0 } else { 1.0 };
        let k = gabor_kernel_3d(pk, frequency, orientation);
        noise[0] += w * k[0];
        noise[1] += w * k[1];
    }
    noise
}

/// Gabor Texture: (Value, Phase, Intensity) at the already scaled point.
/// `orientation` is the 2D angle in `[0]` or the 3D direction.
pub fn gabor(p: [f32; 3], three_d: bool, frequency: f32, anisotropy: f32, orientation_2d: f32, orientation_3d: [f32; 3]) -> (f32, f32, f32) {
    let frequency = frequency.max(0.001);
    let isotropy = 1.0 - anisotropy.clamp(0.0, 1.0);
    let (phasor, sd) = if three_d {
        let l = (orientation_3d[0].powi(2) + orientation_3d[1].powi(2) + orientation_3d[2].powi(2)).sqrt();
        let o = if l > 0.0 { orientation_3d.map(|x| x / l) } else { [0.0, 0.0, 1.0] };
        let cell = p.map(f32::floor);
        let local = [p[0] - cell[0], p[1] - cell[1], p[2] - cell[2]];
        let mut sum = [0.0; 2];
        for off in offsets::<3>(1) {
            let c = add(cell, off);
            let lp = [local[0] - off[0], local[1] - off[1], local[2] - off[2]];
            let v = gabor_cell_3d(c, lp, frequency, isotropy, o);
            sum[0] += v[0];
            sum[1] += v[1];
        }
        let integral = 1.0 / (4.0 * core::f32::consts::SQRT_2);
        (sum, (GABOR_IMPULSES as f32 * 0.5 * integral).sqrt())
    } else {
        let cell = [p[0].floor(), p[1].floor()];
        let local = [p[0] - cell[0], p[1] - cell[1]];
        let mut sum = [0.0; 2];
        for off in offsets::<2>(1) {
            let c = add(cell, off);
            let lp = [local[0] - off[0], local[1] - off[1]];
            let v = gabor_cell_2d(c, lp, frequency, isotropy, orientation_2d);
            sum[0] += v[0];
            sum[1] += v[1];
        }
        (sum, (GABOR_IMPULSES as f32 * 0.5 * 0.25f32).sqrt())
    };
    let norm = 6.0 * sd;
    let value = (phasor[1] / norm) * 0.5 + 0.5;
    let phase = (phasor[1].atan2(phasor[0]) + PI) / TAU;
    let intensity = (phasor[0] * phasor[0] + phasor[1] * phasor[1]).sqrt() / norm;
    (value, phase, intensity)
}

// ---------------------------------------------------------------------------
// Coordinate maps

/// `map_to_tube`: cylindrical (u, v) of a direction about Z.
pub fn map_to_tube(d: [f32; 3]) -> (f32, f32) {
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len > 0.0 {
        ((1.0 - (d[0] / len).atan2(d[1] / len) / PI) * 0.5, (d[2] + 1.0) * 0.5)
    } else {
        (0.0, 0.0)
    }
}

/// `map_to_sphere`: spherical (u, v) of a direction.
pub fn map_to_sphere(d: [f32; 3]) -> (f32, f32) {
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    if len > 0.0 {
        let u = if d[0] == 0.0 && d[1] == 0.0 { 0.0 } else { (1.0 - d[0].atan2(d[1]) / PI) / 2.0 };
        (u, 1.0 - (d[2] / len).clamp(-1.0, 1.0).acos() / PI)
    } else {
        (0.0, 0.0)
    }
}

/// `direction_to_mirrorball`.
pub fn direction_to_mirrorball(d: [f32; 3]) -> (f32, f32) {
    let mut d = d;
    d[1] -= 1.0;
    let div = 2.0 * (-0.5 * d[1]).max(0.0).sqrt();
    if div > 0.0 {
        d = d.map(|x| x / div);
    }
    (0.5 * (d[0] + 1.0), 0.5 * (d[2] + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn hashes_match_lookup3() {
        // Reference values from an independent Python port of Bob Jenkins'
        // lookup3 `hashword` with Blender's initval 13.
        assert_eq!(hash_uint(0), 0x9519dc65);
        assert_eq!(hash_uint(1), 0x4ad7c6a0);
        assert_eq!(hash_uint2(1, 2), 0x1037cb9d);
        assert_eq!(hash_uint3(1, 2, 3), 0xb8d17a0a);
        assert_eq!(hash_uint4(1, 2, 3, 4), 0x0584dec1);
        assert!(close(hash_f3([1.0, 2.0, 3.0]), 0.294_598_13, 1e-7));
        assert!(close(brick_noise(0), 0.263_250_63, 1e-6));
        assert!(close(brick_noise(65537), 0.108_561_56, 1e-6));
        // Perlin and fBm at a non-lattice point, same reference.
        assert!(close(snoise([1.5f32, 3.5, -1.0]), -0.12275, 1e-5));
        assert!(close(noise_fbm([1.5f32, 3.5, -1.0], 2.0, 0.5, 2.0, true), 0.464_928_57, 1e-5));
        assert!(close(noise_fbm([1.5f32, 3.5, -1.0], 2.5, 0.5, 2.0, true), 0.466_097_6, 1e-5));
    }

    #[test]
    fn perlin_vanishes_on_lattice() {
        for p in [[0.0f32, 0.0, 0.0], [1.0, 2.0, -3.0], [5.0, -7.0, 11.0]] {
            assert_eq!(snoise(p), 0.0);
            assert_eq!(noise_fbm(p, 0.0, 0.5, 2.0, true), 0.5);
        }
        assert_eq!(snoise([3.0f32]), 0.0);
        assert_eq!(snoise([3.0f32, -4.0]), 0.0);
        assert_eq!(snoise([3.0f32, -4.0, 1.0, 2.0]), 0.0);
        // Values stay in range and vary.
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for i in 0..2000 {
            let x = i as f32 * 0.137;
            let v = noise_fbm([x, x * 0.7 + 0.3, -x * 0.31], 2.0, 0.5, 2.0, true);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert!(lo > 0.0 && hi < 1.0 && hi - lo > 0.4, "{lo} {hi}");
    }

    #[test]
    fn voronoi_regular_grid() {
        // Randomness 0 puts every feature point at its cell's corner, so F1
        // is the distance to the nearest integer lattice point.
        let p = VoronoiParams {
            scale: 1.0,
            detail: 0.0,
            roughness: 0.5,
            lacunarity: 2.0,
            smoothness: 0.5,
            exponent: 0.5,
            randomness: 0.0,
            max_distance: 0.0,
            normalize: false,
            feature: VoronoiFeature::F1,
            metric: VoronoiMetric::Euclidean,
        };
        let (o, _) = voronoi(p, [0.3f32, 0.4, 0.0]);
        assert!(close(o.distance, 0.5, 1e-6), "{}", o.distance);
        assert_eq!(&o.position[..3], &[0.0, 0.0, 0.0]);
        let (o, _) = voronoi(VoronoiParams { feature: VoronoiFeature::F2, ..p }, [0.3f32, 0.4, 0.0]);
        assert!(close(o.distance, (0.3f32 * 0.3 + 0.6 * 0.6).sqrt(), 1e-6), "{}", o.distance);
        let (o, _) = voronoi(VoronoiParams { metric: VoronoiMetric::Manhattan, ..p }, [0.3f32, 0.4, 0.0]);
        assert!(close(o.distance, 0.7, 1e-6));
        let (o, _) = voronoi(VoronoiParams { metric: VoronoiMetric::Chebychev, ..p }, [0.3f32, 0.4, 0.2]);
        assert!(close(o.distance, 0.4, 1e-6));
        let (o, _) = voronoi(VoronoiParams { feature: VoronoiFeature::DistanceToEdge, ..p }, [0.3f32, 0.4, 0.1]);
        assert!(close(o.distance, 0.1, 1e-6), "{}", o.distance);
        let (_, r) = voronoi(VoronoiParams { feature: VoronoiFeature::NSphereRadius, ..p }, [0.3f32, 0.4, 0.1]);
        assert!(close(r, 0.5, 1e-6));
        // 1D: distance to the nearest integer.
        let (o, _) = voronoi(p, [2.25f32]);
        assert!(close(o.distance, 0.25, 1e-6));
        // Normalised F1 divides by the largest possible distance.
        let (o, _) = voronoi(VoronoiParams { normalize: true, ..p }, [0.5f32, 0.5]);
        assert!(close(o.distance, 0.5f32.hypot(0.5) / 0.5f32.hypot(0.5), 1e-5));
        // Randomness 1: the point is inside its own cell's neighbourhood.
        let (o, _) = voronoi(VoronoiParams { randomness: 1.0, ..p }, [7.3f32, -2.1, 0.4]);
        assert!(o.distance >= 0.0 && o.distance < 1.8);
        let c = o.color;
        assert!(c.iter().all(|x| (0.0..=1.0).contains(x)));
    }

    #[test]
    fn checker_gradient_wave_magic_brick() {
        assert_eq!(checker([0.5, 0.5, 0.5]), 0.0);
        assert_eq!(checker([1.5, 0.5, 0.5]), 1.0);
        assert_eq!(checker([1.5, 1.5, 0.5]), 0.0);
        assert_eq!(checker([1.5, 1.5, 1.5]), 1.0);
        assert_eq!(gradient([0.25, 0.0, 0.0], 0), 0.25);
        assert_eq!(gradient([0.5, 0.0, 0.0], 1), 0.25);
        assert!(close(gradient([0.5, 0.0, 0.0], 2), 0.5, 1e-6));
        assert!(close(gradient([0.2, 0.4, 0.0], 3), 0.3, 1e-6));
        assert!(close(gradient([0.0, 1.0, 0.0], 4), 0.75, 1e-6));
        assert!(close(gradient([0.5, 0.0, 0.0], 5), 0.25, 1e-6));
        assert!(close(gradient([0.5, 0.0, 0.0], 6), 0.5, 1e-6));
        // Sine bands: 0.5 + 0.5 sin(20 x - pi/2) (with the precision nudge).
        let x = 0.1f32;
        let px = (x + 0.000001) * 0.999999;
        assert!(close(wave(WaveType::Bands, 0, 0, [x, 0.0, 0.0], 0.0, 2.0, 1.0, 0.5, 0.0), 0.5 + 0.5 * (px * 20.0 - FRAC_PI_2).sin(), 1e-6));
        let saw = wave(WaveType::Bands, 0, 1, [x, 0.0, 0.0], 0.0, 2.0, 1.0, 0.5, 0.0);
        assert!(close(saw, (px * 20.0 / TAU).fract(), 1e-6));
        let rings = wave(WaveType::Rings, 3, 1, [0.3, 0.4, 0.0], 0.0, 2.0, 1.0, 0.5, 0.0);
        assert!(close(rings, (0.5f32 * 20.0 / TAU).fract(), 1e-4));
        // Magic depth 0, distortion 1: closed form.
        let p = [0.1f32, 0.2, 0.3];
        let m = magic(p, 0, 1.0);
        assert!(close(m[0], 0.5 - (0.6f32 * 5.0).sin() / 2.0, 1e-6));
        assert!(close(m[1], 0.5 - ((-0.1f32 + 0.2 - 0.3) * 5.0).cos() / 2.0, 1e-6));
        // Brick: mortar along the row boundary, none in the middle.
        let b = Brick { mortar_size: 0.02, mortar_smooth: 0.0, bias: 0.0, brick_width: 0.5, row_height: 0.25, offset: 0.5, offset_frequency: 2, squash: 1.0, squash_frequency: 2 };
        assert_eq!(brick([0.1, 0.01, 0.0], &b).1, 1.0);
        assert_eq!(brick([0.1, 0.1, 0.0], &b).1, 0.0);
        let t = brick([0.1, 0.1, 0.0], &b).0;
        assert!((0.0..=1.0).contains(&t));
        // Row 0 is offset by half a brick: x = 0.2 sits in brick 0 at 0.45.
        assert_eq!(brick([0.2, 0.1, 0.0], &b).0, brick([0.2 - 0.04, 0.1, 0.0], &b).0);
    }

    #[test]
    fn white_noise_is_the_hash() {
        let (v, c) = white_noise([0.25f32, 0.5, 0.75]);
        assert_eq!(v, hash_f3([0.25, 0.5, 0.75]));
        assert_eq!(c[0], v);
        let (v1, _) = white_noise([0.25f32]);
        assert_eq!(v1, hash_f1(0.25));
    }
}
