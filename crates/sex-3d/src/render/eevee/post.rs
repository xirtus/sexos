// SPDX-License-Identifier: GPL-3.0-only
//! EEVEE's screen-space passes over one sample: subsurface scattering,
//! screen-space reflections (and the depth-buffer march refraction uses),
//! depth of field and bloom.
//!
//! Every pass reads a snapshot and writes each pixel (or row) from one
//! worker, so the output never depends on the thread count. Random
//! rotations come from a hash of the pixel and the sample index, so the
//! noise changes per TAA sample and averages out in the accumulation.
use super::hash2;
use crate::math::Vec3;
use crate::render::parallel_for_each;
use crate::render::raster::{Layout, RasterCamera, TILE, TILE_PX};
use crate::render::sampling::onb;
use core::f32::consts::TAU;

#[inline]
fn rand01(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

// ---------------------------------------------------------------- SSS

/// A pixel's subsurface part: its scattered irradiance (radiance per unit
/// albedo), the albedo it is multiplied by, and the radius per channel.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SssPx {
    pub irr: Vec3,
    pub albedo: Vec3,
    pub radius: Vec3,
}

/// Blur the subsurface irradiance over the surface (a Gaussian per channel
/// whose width is the radius, taps on a disk in screen space, weighted by
/// their 3D distance so other surfaces do not bleed in) and add it back.
#[allow(clippy::too_many_arguments)]
pub(super) fn subsurface(color: &mut [[f32; 4]], sss: &[SssPx], depth: &[f32], layout: &Layout, cam: &RasterCamera, samples: u32, sample: u32, threads: usize) {
    let taps = samples.clamp(1, 64) as usize;
    let pos = |x: usize, y: usize| cam.unproject(x, y, depth[layout.index(x, y)]);
    parallel_for_each(color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
        let (x0, y0, x1, y1) = layout.tile_rect(t);
        for y in y0..y1 {
            for x in x0..x1 {
                let i = t * TILE_PX + (y % TILE) * TILE + x % TILE;
                let s = &sss[i];
                if s.albedo == Vec3::ZERO {
                    continue;
                }
                let rmax = s.radius.max_elem();
                let p = pos(x, y);
                let dist = if cam.ortho { 1.0 } else { p.length().max(1e-4) };
                let r_px = (rmax * cam.scale / dist).min(64.0);
                let mut sum = s.irr;
                let mut wsum = Vec3::ONE;
                if rmax > 0.0 && r_px >= 0.5 {
                    let sig = s.radius.max(Vec3::splat(rmax * 1e-3)) * 0.5;
                    let inv2 = Vec3::new(0.5 / (sig.x * sig.x), 0.5 / (sig.y * sig.y), 0.5 / (sig.z * sig.z));
                    let h = hash2(x as u32, y as u32, sample.wrapping_mul(0x51ed_270b));
                    let rot = rand01(h) * TAU;
                    for k in 0..taps {
                        // Golden-angle spiral, uniform over the disk.
                        let u = (k as f32 + 0.5) / taps as f32;
                        let a = rot + k as f32 * 2.399_963;
                        let r = u.sqrt() * r_px;
                        let (qx, qy) = (x as f32 + 0.5 + a.cos() * r, y as f32 + 0.5 + a.sin() * r);
                        if !(qx >= 0.0 && qy >= 0.0 && qx < layout.w as f32 && qy < layout.h as f32) {
                            continue;
                        }
                        let (qx, qy) = (qx as usize, qy as usize);
                        let j = layout.index(qx, qy);
                        let q = &sss[j];
                        if q.albedo == Vec3::ZERO {
                            continue;
                        }
                        let d2 = (pos(qx, qy) - p).length_squared();
                        let w = Vec3::new((-d2 * inv2.x).exp(), (-d2 * inv2.y).exp(), (-d2 * inv2.z).exp());
                        sum += w.mul_elem(q.irr);
                        wsum += w;
                    }
                }
                let blurred = Vec3::new(sum.x / wsum.x, sum.y / wsum.y, sum.z / wsum.z);
                let add = s.albedo.mul_elem(blurred);
                let c = &mut col[(y % TILE) * TILE + x % TILE];
                c[0] += add.x;
                c[1] += add.y;
                c[2] += add.z;
            }
        }
    });
}

// ---------------------------------------------------------------- SSR

/// A pixel's reflection: the reflectance of its glossy lobes, the probe
/// light they already received (which a screen-space hit replaces), the
/// world-space normal and the roughness.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SpecPx {
    pub weight: Vec3,
    pub probe: Vec3,
    pub n: Vec3,
    pub rough: f32,
}

/// Screen-space reflection settings.
#[derive(Clone, Copy, Debug)]
pub(super) struct Ssr {
    pub max_roughness: f32,
    pub thickness: f32,
    pub fade: f32,
    pub max_distance: f32,
}

/// March view-space ray `o` + t `d` through the depth buffer. Returns the
/// hit pixel and how much of the hit to trust (screen-edge fade).
#[allow(clippy::too_many_arguments)]
pub(super) fn march(depth: &[f32], layout: &Layout, cam: &RasterCamera, o: Vec3, d: Vec3, max_dist: f32, thickness: f32, fade: f32, jit: f32) -> Option<(usize, usize, f32)> {
    // Clip the ray to the near plane.
    let mut len = max_dist;
    if !cam.ortho && d.z > 0.0 {
        len = len.min((-cam.near - o.z) / d.z * 0.999);
    }
    if !(len > 1e-5) {
        return None;
    }
    let (sx0, sy0, k0) = cam.project(o)?;
    let (sx1, sy1, k1) = cam.project(o + d * len)?;
    let (w, h) = (layout.w as f32, layout.h as f32);
    // Clip the screen segment to the frame (keys stay linear in screen space).
    let mut tmax = 1.0f32;
    for (a, b, lim) in [(sx0, sx1, w), (sy0, sy1, h)] {
        let d = b - a;
        if d > 0.0 && b > lim {
            tmax = tmax.min((lim - a) / d);
        } else if d < 0.0 && b < 0.0 {
            tmax = tmax.min(-a / d);
        }
    }
    if !(tmax > 0.0) {
        return None;
    }
    let (sx1, sy1, k1) = (sx0 + (sx1 - sx0) * tmax, sy0 + (sy1 - sy0) * tmax, k0 + (k1 - k0) * tmax);
    // Steps of about 3 pixels, 8 to 64 of them.
    let span = ((sx1 - sx0).abs().max((sy1 - sy0).abs()) / 3.0).clamp(8.0, 64.0) as usize;
    let at = |t: f32| (sx0 + (sx1 - sx0) * t, sy0 + (sy1 - sy0) * t, k0 + (k1 - k0) * t);
    // The depth key at a screen position: bilinear between pixel centres
    // (exact on planes, so a ray leaving a surface at a grazing angle does
    // not hit the pixel it starts on), nearest across depth edges.
    let key_at = |x: f32, y: f32| -> f32 {
        let (px, py) = (x as usize, y as usize);
        let k = depth[layout.index(px, py)];
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        if k == f32::NEG_INFINITY || x0 < 0.0 || y0 < 0.0 || x0 + 1.0 >= w || y0 + 1.0 >= h {
            return k;
        }
        let (ix, iy) = (x0 as usize, y0 as usize);
        let q = [depth[layout.index(ix, iy)], depth[layout.index(ix + 1, iy)], depth[layout.index(ix, iy + 1)], depth[layout.index(ix + 1, iy + 1)]];
        let (lo, hi) = q.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
        if lo == f32::NEG_INFINITY || (hi - lo).abs() > 0.05 * k.abs().max(1e-6) * if cam.ortho { 1e3 } else { 1.0 } {
            return k;
        }
        let (tx, ty) = (fx - x0, fy - y0);
        (q[0] * (1.0 - tx) + q[1] * tx) * (1.0 - ty) + (q[2] * (1.0 - tx) + q[3] * tx) * ty
    };
    // Behind the surface (smaller key = farther) by less than `slack`.
    let test = |t: f32, slack: f32| -> Option<(bool, usize, usize)> {
        let (x, y, k) = at(t);
        if !(x >= 0.0 && y >= 0.0 && x < w && y < h) {
            return None;
        }
        let (px, py) = (x as usize, y as usize);
        let ks = key_at(x, y);
        if ks == f32::NEG_INFINITY {
            return Some((false, px, py));
        }
        let behind = k < ks && cam.depth_of(k) - cam.depth_of(ks) < slack;
        Some((behind, px, py))
    };
    // Start a pixel away from the origin.
    let len_px = (sx1 - sx0).abs().max((sy1 - sy0).abs()).max(1e-3);
    let t0 = (1.0 / len_px).min(1.0);
    let mut prev = t0;
    let mut prev_depth = cam.depth_of(at(t0).2);
    for i in 0..span {
        let t = t0 + (1.0 - t0) * (i as f32 + jit) / span as f32;
        // A step can cross more than the thickness: allow what it travelled.
        let dz = cam.depth_of(at(t).2);
        let slack = thickness.max((dz - prev_depth).abs() * 1.01);
        prev_depth = dz;
        let (hit, _, _) = test(t, slack)?;
        if hit {
            // Refine between the last miss and this hit.
            let (mut a, mut b) = (prev, t);
            for _ in 0..5 {
                let m = 0.5 * (a + b);
                match test(m, slack) {
                    Some((true, _, _)) => b = m,
                    _ => a = m,
                }
            }
            let (_, px, py) = test(b, slack)?;
            let (x, y, _) = at(b);
            let edge = (x.min(w - x) / (fade * w).max(1e-3)).min(y.min(h - y) / (fade * h).max(1e-3)).clamp(0.0, 1.0);
            return Some((px, py, edge));
        }
        prev = t;
    }
    None
}

/// Replace the probe reflection of glossy pixels by what a ray through the
/// depth buffer hits in `lit` (this sample's lit opaque surfaces).
#[allow(clippy::too_many_arguments)]
pub(super) fn reflections(color: &mut [[f32; 4]], lit: &[[f32; 4]], spec: &[SpecPx], depth: &[f32], normals: &[Vec3], layout: &Layout, cam: &RasterCamera, p: &Ssr, sample: u32, threads: usize) {
    parallel_for_each(color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
        let (x0, y0, x1, y1) = layout.tile_rect(t);
        for y in y0..y1 {
            for x in x0..x1 {
                let k = (y % TILE) * TILE + x % TILE;
                let i = t * TILE_PX + k;
                let s = &spec[i];
                if s.weight == Vec3::ZERO || s.rough > p.max_roughness {
                    continue;
                }
                let key = depth[i];
                if key == f32::NEG_INFINITY {
                    continue;
                }
                let pv = cam.unproject(x, y, key);
                let n = cam.rotate(s.n).normalize();
                let v = if cam.ortho { -Vec3::Z } else { pv.normalize() };
                let h = hash2(x as u32, y as u32, sample.wrapping_mul(0x2c1b_3c6d) ^ 0x5bd1_e995);
                let (u1, u2, jit) = (rand01(h), rand01(h.wrapping_mul(0x9E37_79B9) ^ (h >> 13)), rand01(hash2(y as u32, x as u32, sample ^ 0xa511_e9b3)));
                // A GGX microfacet normal (roughness jitter that averages out).
                let a = (s.rough * s.rough).max(1e-4);
                let hn = if a > 2e-3 {
                    let cos_t = ((1.0 - u1) / (1.0 + (a * a - 1.0) * u1)).max(0.0).sqrt();
                    let sin_t = (1.0 - cos_t * cos_t).max(0.0).sqrt();
                    let (tx, ty) = onb(n);
                    let phi = TAU * u2;
                    (tx * (sin_t * phi.cos()) + ty * (sin_t * phi.sin()) + n * cos_t).normalize()
                } else {
                    n
                };
                let mut r = v - hn * (2.0 * v.dot(hn));
                if r.dot(n) <= 0.0 {
                    r = v - n * (2.0 * v.dot(n));
                }
                let dist = if cam.ortho { 1.0 } else { pv.length() };
                let o = pv + n * (dist * 2e-3 + 1e-4);
                let Some((hx, hy, edge)) = march(depth, layout, cam, o, r, p.max_distance, p.thickness, p.fade, jit) else { continue };
                let j = layout.index(hx, hy);
                // Back faces of what we hit are not seen.
                if normals[j].dot(r) > 0.0 {
                    continue;
                }
                // Fade out towards the roughness limit.
                let rough_fade = ((p.max_roughness - s.rough) / (0.1 * p.max_roughness.max(0.01))).clamp(0.0, 1.0);
                let f = edge * rough_fade;
                if f <= 0.0 {
                    continue;
                }
                let hc = lit[j];
                let hit = Vec3::new(hc[0], hc[1], hc[2]);
                let delta = (s.weight.mul_elem(hit) - s.probe) * f;
                let c = &mut col[k];
                c[0] = (c[0] + delta.x).max(0.0);
                c[1] = (c[1] + delta.y).max(0.0);
                c[2] = (c[2] + delta.z).max(0.0);
            }
        }
    });
}

// ---------------------------------------------------------------- DOF

/// Thin-lens depth of field for the gather filter.
#[derive(Clone, Copy, Debug)]
pub(super) struct DofBlur {
    /// Aperture radius times the camera's pixel scale.
    pub r_scale: f32,
    pub focus: f32,
    /// Largest circle of confusion radius in pixels.
    pub max_radius: f32,
}

impl DofBlur {
    /// Circle of confusion radius (pixels) for a depth key.
    #[inline]
    fn coc(&self, cam: &RasterCamera, key: f32) -> f32 {
        let c = if cam.ortho {
            if key == f32::NEG_INFINITY {
                self.max_radius
            } else {
                self.r_scale * (1.0 - cam.depth_of(key) / self.focus).abs()
            }
        } else if key == f32::NEG_INFINITY {
            self.r_scale / self.focus
        } else {
            self.r_scale * (1.0 / self.focus - key).abs()
        };
        c.min(self.max_radius)
    }
}

/// Gather depth of field: every pixel collects the neighbours whose circle
/// of confusion covers it. Out-of-focus foreground spreads over what is
/// behind it; background never spreads over sharper foreground.
pub(super) fn depth_of_field(color: &mut Vec<[f32; 4]>, depth: &[f32], layout: &Layout, cam: &RasterCamera, p: &DofBlur, sample: u32, threads: usize, scratch: &mut Vec<[f32; 4]>) {
    let n = layout.len();
    let mut coc = vec![0.0f32; n];
    let mut dist = vec![0.0f32; n];
    for i in 0..n {
        coc[i] = p.coc(cam, depth[i]);
        dist[i] = if depth[i] == f32::NEG_INFINITY { f32::INFINITY } else { cam.depth_of(depth[i]) };
    }
    // Largest CoC per tile, dilated over the tiles it can reach.
    let tiles = layout.tiles();
    let tmax: Vec<f32> = (0..tiles).map(|t| coc[t * TILE_PX..(t + 1) * TILE_PX].iter().copied().fold(0.0, f32::max)).collect();
    let global = tmax.iter().copied().fold(0.0, f32::max);
    if global < 0.5 {
        return;
    }
    let reach = (global / TILE as f32).ceil() as isize + 1;
    let dil: Vec<f32> = (0..tiles)
        .map(|t| {
            let (tx, ty) = ((t % layout.tx) as isize, (t / layout.tx) as isize);
            let mut m = 0.0f32;
            for dy in -reach..=reach {
                for dx in -reach..=reach {
                    let (nx, ny) = (tx + dx, ty + dy);
                    if nx < 0 || ny < 0 || nx >= layout.tx as isize || ny >= layout.ty as isize {
                        continue;
                    }
                    let c = tmax[ny as usize * layout.tx + nx as usize];
                    // Can a CoC of this tile reach ours?
                    let gap = ((dx.abs() - 1).max(0).max((dy.abs() - 1).max(0)) * TILE as isize) as f32;
                    if c >= gap {
                        m = m.max(c);
                    }
                }
            }
            m
        })
        .collect();
    scratch.clear();
    scratch.extend_from_slice(color);
    let src = &scratch[..];
    const RINGS: usize = 4;
    parallel_for_each(color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
        let r = dil[t];
        if r < 0.5 {
            return;
        }
        let rings = ((r / 3.0).ceil() as usize).clamp(1, RINGS);
        let ntaps = 1 + 4 * rings * (rings + 1);
        let (x0, y0, x1, y1) = layout.tile_rect(t);
        for y in y0..y1 {
            for x in x0..x1 {
                let k = (y % TILE) * TILE + x % TILE;
                let i = t * TILE_PX + k;
                let (cc, dc) = (coc[i], dist[i]);
                let h = hash2(x as u32, y as u32, sample.wrapping_mul(0x7feb_352d));
                let rot = rand01(h) * TAU;
                let rj = rand01(h.wrapping_mul(0x846c_a68b));
                let (mut bg, mut bgw) = ([0.0f32; 4], 0.0f32);
                let (mut fg, mut fgw) = ([0.0f32; 4], 0.0f32);
                // Each tap stands for this much of the disk (in pixels²/π).
                let tap_area = r * r / ntaps as f32;
                let mut add = |q: usize, s: f32| {
                    let (cq, dq) = (coc[q], dist[q]);
                    let c = &src[q];
                    let front = dq < dc - 0.02 * dc.min(1e6);
                    if front {
                        let cover = (cq - s + 0.5).clamp(0.0, 1.0);
                        if cover > 0.0 {
                            let w = cover * tap_area / cq.max(0.5).powi(2);
                            for ch in 0..4 {
                                fg[ch] += c[ch] * w;
                            }
                            fgw += w;
                        }
                    } else {
                        let ce = if dq > dc { cq.min(cc) } else { cq };
                        let cover = (ce - s + 0.5).clamp(0.0, 1.0);
                        if cover > 0.0 {
                            let w = cover / ce.max(0.5).powi(2);
                            for ch in 0..4 {
                                bg[ch] += c[ch] * w;
                            }
                            bgw += w;
                        }
                    }
                };
                add(i, 0.0);
                for ring in 1..=rings {
                    let count = 8 * ring;
                    let rad = (ring as f32 - 0.5 + rj) / rings as f32 * r;
                    for j in 0..count {
                        let a = rot + (j as f32 + 0.5 * (ring & 1) as f32) * TAU / count as f32;
                        let (qx, qy) = (x as f32 + 0.5 + a.cos() * rad, y as f32 + 0.5 + a.sin() * rad);
                        if !(qx >= 0.0 && qy >= 0.0 && qx < layout.w as f32 && qy < layout.h as f32) {
                            continue;
                        }
                        add(layout.index(qx as usize, qy as usize), rad);
                    }
                }
                let alpha = fgw.min(1.0);
                let fgs = if fgw > 0.0 { alpha / fgw } else { 0.0 };
                let bgs = if bgw > 0.0 { (1.0 - alpha) / bgw } else { 0.0 };
                let o = &mut col[k];
                for ch in 0..4 {
                    o[ch] = fg[ch] * fgs + bg[ch] * bgs;
                }
            }
        }
    });
}

// ---------------------------------------------------------------- bloom

/// EEVEE's bloom settings.
#[derive(Clone, Copy, Debug)]
pub(super) struct BloomParams {
    pub threshold: f32,
    pub knee: f32,
    pub radius: f32,
    pub color: Vec3,
    pub intensity: f32,
    pub clamp: f32,
}

struct Img {
    w: usize,
    h: usize,
    px: Vec<Vec3>,
}

impl Img {
    #[inline]
    fn at(&self, x: isize, y: isize) -> Vec3 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.px[y * self.w + x]
    }
    /// Bilinear sample at continuous pixel coordinates (centres at +0.5).
    #[inline]
    fn bilinear(&self, x: f32, y: f32) -> Vec3 {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.at(x0, y0) * (1.0 - tx) + self.at(x0 + 1, y0) * tx;
        let b = self.at(x0, y0 + 1) * (1.0 - tx) + self.at(x0 + 1, y0 + 1) * tx;
        a * (1.0 - ty) + b * ty
    }
    /// Half size, 2x2 box (with a tent around it).
    fn down(&self, threads: usize) -> Img {
        let (w, h) = (self.w.div_ceil(2).max(1), self.h.div_ceil(2).max(1));
        let mut px = vec![Vec3::ZERO; w * h];
        parallel_for_each(px.chunks_mut(w).enumerate(), threads, |(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let (sx, sy) = (2 * x as isize, 2 * y as isize);
                // 4x4 tent: inner 2x2 weighted 4, ring weighted 1/2/1.
                let mut s = (self.at(sx, sy) + self.at(sx + 1, sy) + self.at(sx, sy + 1) + self.at(sx + 1, sy + 1)) * 4.0;
                s += (self.at(sx - 1, sy) + self.at(sx - 1, sy + 1) + self.at(sx + 2, sy) + self.at(sx + 2, sy + 1) + self.at(sx, sy - 1) + self.at(sx + 1, sy - 1) + self.at(sx, sy + 2) + self.at(sx + 1, sy + 2)) * 2.0;
                s += self.at(sx - 1, sy - 1) + self.at(sx + 2, sy - 1) + self.at(sx - 1, sy + 2) + self.at(sx + 2, sy + 2);
                *o = s * (1.0 / 36.0);
            }
        });
        Img { w, h, px }
    }
}

/// Add bloom (a soft glow of the parts brighter than the threshold) to the
/// sample, as EEVEE's: prefilter with a soft knee, a pyramid of blurs whose
/// depth the radius sets, then add `color * intensity` of it.
pub(super) fn bloom(color: &mut [[f32; 4]], layout: &Layout, p: &BloomParams, threads: usize) {
    let (w, h) = (layout.w, layout.h);
    if w < 2 || h < 2 || p.intensity <= 0.0 {
        return;
    }
    // Prefilter at full size, row-major.
    let knee = p.knee.max(1e-5);
    let mut base = Img { w, h, px: vec![Vec3::ZERO; w * h] };
    parallel_for_each(base.px.chunks_mut(w).enumerate(), threads, |(y, row)| {
        for (x, o) in row.iter_mut().enumerate() {
            let c = color[layout.index(x, y)];
            let mut v = Vec3::new(c[0], c[1], c[2]);
            if !(v.x.is_finite() && v.y.is_finite() && v.z.is_finite()) {
                continue;
            }
            if p.clamp > 0.0 {
                let m = v.max_elem();
                if m > p.clamp {
                    v = v * (p.clamp / m);
                }
            }
            let br = v.max_elem();
            let rq = (br - p.threshold + knee).clamp(0.0, 2.0 * knee);
            let rq = rq * rq / (4.0 * knee + 1e-5);
            let contrib = (rq.max(br - p.threshold)) / br.max(1e-5);
            *o = v * contrib.max(0.0);
        }
    });
    let maxdim = w.max(h) as f32;
    let logm = maxdim.log2() + p.radius.max(0.0) - 8.0;
    let iters = (logm.floor() as i32).clamp(1, 12) as usize;
    let spread = (0.5 + logm - iters as f32).clamp(0.5, 1.5);
    let mut pyr = vec![base.down(threads)];
    for _ in 1..iters {
        let last = pyr.last().map(|l| l.down(threads));
        match last {
            Some(n) if n.w >= 2 && n.h >= 2 => pyr.push(n),
            _ => break,
        }
    }
    // Upsample: each level adds a tent-filtered copy of the one below.
    while pyr.len() > 1 {
        let low = pyr.pop().unwrap_or(Img { w: 1, h: 1, px: vec![Vec3::ZERO] });
        let Some(high) = pyr.last_mut() else { break };
        let (hw, sx, sy) = (high.w, low.w as f32 / high.w as f32, low.h as f32 / high.h as f32);
        parallel_for_each(high.px.chunks_mut(hw).enumerate(), threads, |(y, row)| {
            for (x, o) in row.iter_mut().enumerate() {
                let (cx, cy) = ((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy);
                let d = spread;
                let mut s = low.bilinear(cx, cy) * 4.0;
                s += (low.bilinear(cx - d, cy) + low.bilinear(cx + d, cy) + low.bilinear(cx, cy - d) + low.bilinear(cx, cy + d)) * 2.0;
                s += low.bilinear(cx - d, cy - d) + low.bilinear(cx + d, cy - d) + low.bilinear(cx - d, cy + d) + low.bilinear(cx + d, cy + d);
                *o += s * (1.0 / 16.0);
            }
        });
    }
    let top = &pyr[0];
    let (sx, sy) = (top.w as f32 / w as f32, top.h as f32 / h as f32);
    let tint = p.color * p.intensity;
    parallel_for_each(color.chunks_mut(TILE_PX).enumerate(), threads, |(t, col)| {
        let (x0, y0, x1, y1) = layout.tile_rect(t);
        for y in y0..y1 {
            for x in x0..x1 {
                let b = top.bilinear((x as f32 + 0.5) * sx, (y as f32 + 0.5) * sy).mul_elem(tint);
                let c = &mut col[(y % TILE) * TILE + x % TILE];
                c[0] += b.x;
                c[1] += b.y;
                c[2] += b.z;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloom_spreads_a_bright_pixel_and_keeps_dim_ones() {
        let layout = Layout::new(64, 64);
        let mut c = vec![[0.1f32, 0.1, 0.1, 1.0]; layout.len()];
        c[layout.index(32, 32)] = [500.0, 500.0, 500.0, 1.0];
        let p = BloomParams { threshold: 0.8, knee: 0.5, radius: 6.5, color: Vec3::ONE, intensity: 0.05, clamp: 0.0 };
        let mut a = c.clone();
        bloom(&mut a, &layout, &p, 1);
        let mut b = c.clone();
        bloom(&mut b, &layout, &p, 3);
        assert_eq!(a, b);
        assert!(a[layout.index(36, 32)][0] > 0.1 + 1e-3);
        // Far corners get (almost) nothing; the dim pixels stay below threshold.
        assert!((a[layout.index(0, 0)][0] - 0.1).abs() < 0.05);
    }
}
