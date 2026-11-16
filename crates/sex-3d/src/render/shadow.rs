// SPDX-License-Identifier: GPL-3.0-only
//! Shadow maps for the rasterized engines: a depth map rendered from a
//! light's camera (orthographic for sun lights and the Workbench light,
//! perspective for spot lights and the six faces of a point light's cube)
//! with the shared tiled rasterizer, and filtered lookups (rotated
//! percentage-closer filtering over a disk, with normal-offset and
//! slope-scaled bias so lit surfaces do not shadow themselves).
use super::raster::{Bins, Facing, Layout, RasterCamera};
use crate::math::Vec3;

#[derive(Clone, Debug)]
pub(crate) struct ShadowMap {
    pub cam: RasterCamera,
    layout: Layout,
    depth: Vec<f32>,
    /// World size of a texel: absolute (ortho) or per unit distance (perspective).
    texel: f32,
}

/// Disk taps for PCF (unit radius), a 12-point Poisson set.
const TAPS: [(f32, f32); 12] = [
    (-0.326, -0.406),
    (-0.840, -0.074),
    (-0.696, 0.457),
    (-0.203, 0.621),
    (0.962, -0.195),
    (0.473, -0.480),
    (0.519, 0.767),
    (0.185, -0.893),
    (0.507, 0.064),
    (0.896, 0.412),
    (-0.322, -0.933),
    (-0.792, -0.598),
];

impl ShadowMap {
    /// Render the nearest depth of triangles `tri(i)` for `i` in `0..count`
    /// (world-space corners; `None` skips) as seen by `cam`.
    pub fn render(cam: RasterCamera, threads: usize, count: usize, bins: &mut Bins<()>, tri: impl Fn(usize) -> Option<[Vec3; 3]> + Sync) -> ShadowMap {
        let layout = Layout::new(cam.width as usize, cam.height as usize);
        let pieces = (threads * 2).clamp(1, 64).min(count.div_ceil(2048).max(1));
        bins.begin(layout, pieces);
        let n = bins.pieces.len();
        let per = count.div_ceil(n);
        bins.setup(threads, |k, piece| {
            for i in k * per..((k + 1) * per).min(count) {
                let Some(p) = tri(i) else { continue };
                cam.clip_emit(p.map(|q| cam.to_view(q)), |v, b| {
                    piece.push(v, b, Facing::Both, ());
                });
            }
        });
        let mut depth = Vec::new();
        bins.depth_only(&mut depth, threads);
        let texel = 1.0 / cam.scale;
        ShadowMap { cam, layout, depth, texel }
    }

    /// World size of a texel at `p`.
    fn texel_at(&self, p: Vec3) -> f32 {
        if self.cam.ortho {
            self.texel
        } else {
            self.texel * (-self.cam.to_view(p).z).max(self.cam.near)
        }
    }

    /// Fraction of light reaching world point `p` with geometric normal `n`
    /// (unit, facing the light side or not): 1 lit, 0 shadowed. `radius`
    /// is the PCF radius in texels (0 = one tap) and `rot` rotates the taps
    /// (vary it per pixel and sample for noise that averages out).
    pub fn lit(&self, p: Vec3, n: Vec3, radius: f32, rot: f32) -> f32 {
        let texel = self.texel_at(p);
        // Normal offset towards the light side of the surface.
        let l = if self.cam.ortho { self.cam.back } else { (self.cam.eye - p).normalize() };
        let ndl = n.dot(l);
        let n = if ndl < 0.0 { -n } else { n };
        let cos = ndl.abs().clamp(0.05, 1.0);
        let sin = (1.0 - cos * cos).sqrt();
        let q = p + n * (texel * (1.0 + 1.5 * sin));
        let v = self.cam.to_view(q);
        let Some((sx, sy, key)) = self.cam.project(v) else { return 1.0 };
        let depth = self.cam.depth_of(key);
        // Slope-scaled depth bias (world units along the light axis).
        let bias = texel * (0.5 + (2.0 + radius) * (sin / cos).min(4.0));
        let (w, h) = (self.layout.w as f32, self.layout.h as f32);
        let lookup = |x: f32, y: f32| -> f32 {
            if !(x >= 0.0 && y >= 0.0 && x < w && y < h) {
                return 1.0;
            }
            let k = self.depth[self.layout.index(x as usize, y as usize)];
            if k == f32::NEG_INFINITY {
                return 1.0;
            }
            if depth <= self.cam.depth_of(k) + bias { 1.0 } else { 0.0 }
        };
        if radius <= 0.0 {
            return lookup(sx, sy);
        }
        let (s, c) = rot.sin_cos();
        let mut sum = 0.0;
        for &(tx, ty) in &TAPS {
            let (ox, oy) = ((tx * c - ty * s) * radius, (tx * s + ty * c) * radius);
            sum += lookup(sx + ox, sy + oy);
        }
        sum / TAPS.len() as f32
    }
}
