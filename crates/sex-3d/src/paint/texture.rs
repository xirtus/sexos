// SPDX-License-Identifier: GPL-3.0-only
//! Texture Paint: brushes that paint an image, either through a mesh's UVs
//! from the 3D view (projection painting, with occlusion, backface culling
//! and seam bleed) or directly in the Image Editor. Undo keeps only the
//! 64x64 tiles a stroke touched.
use super::{blend, mix4, BlendMode};
use crate::math::{Vec2, Vec3};
use crate::mesh::Mesh;
use crate::scene::Image;
use crate::sculpt::Curve;
use crate::undo::Change;
use std::collections::HashMap;

/// Texture Paint brushes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TexBrush {
    #[default]
    Draw,
    Soften,
    Smear,
    Clone,
    Fill,
    Mask,
}

impl TexBrush {
    pub const ALL: [TexBrush; 6] = [TexBrush::Draw, TexBrush::Soften, TexBrush::Smear, TexBrush::Clone, TexBrush::Fill, TexBrush::Mask];
    pub fn label(self) -> &'static str {
        match self {
            TexBrush::Draw => "Draw",
            TexBrush::Soften => "Soften",
            TexBrush::Smear => "Smear",
            TexBrush::Clone => "Clone",
            TexBrush::Fill => "Fill",
            TexBrush::Mask => "Mask",
        }
    }
}

/// Texture Paint settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TpSettings {
    pub brush: TexBrush,
    /// Linear RGB.
    pub color: [f32; 3],
    pub secondary: [f32; 3],
    pub strength: f32,
    /// Radius in screen pixels.
    pub radius: f32,
    pub blend: BlendMode,
    pub curve: Curve,
    /// Pixels painted past UV island borders.
    pub bleed: u32,
    /// Only paint what is visible (Blender's Occlude).
    pub occlude: bool,
    pub backface_culling: bool,
    /// Respect the stencil mask the Mask brush paints.
    pub use_mask: bool,
    /// Fill: color difference that still counts as the same region (2D).
    pub fill_threshold: f32,
}

impl Default for TpSettings {
    fn default() -> Self {
        TpSettings {
            brush: TexBrush::Draw,
            color: [1.0, 1.0, 1.0],
            secondary: [0.0, 0.0, 0.0],
            strength: 1.0,
            radius: 50.0,
            blend: BlendMode::Mix,
            curve: Curve::Smooth,
            bleed: 2,
            occlude: true,
            backface_culling: true,
            use_mask: true,
            fill_threshold: 0.2,
        }
    }
}

pub const TILE: u32 = 64;

/// Original tiles of an image, saved before the stroke first writes them.
#[derive(Clone, Debug, Default)]
pub struct TileUndo {
    pub image: usize,
    saved: HashMap<(u32, u32), Vec<[f32; 4]>>,
}

impl TileUndo {
    pub fn new(image: usize) -> TileUndo {
        TileUndo { image, saved: HashMap::new() }
    }
    /// Remember the tile holding pixel (x, y) if not yet saved.
    pub fn save(&mut self, img: &Image, x: u32, y: u32) {
        let k = (x / TILE, y / TILE);
        if self.saved.contains_key(&k) {
            return;
        }
        let (x0, y0) = (k.0 * TILE, k.1 * TILE);
        let (w, h) = (TILE.min(img.width - x0), TILE.min(img.height - y0));
        let mut px = Vec::with_capacity((w * h) as usize);
        for yy in y0..y0 + h {
            let row = (yy * img.width) as usize;
            px.extend_from_slice(&img.pixels[row + x0 as usize..row + (x0 + w) as usize]);
        }
        self.saved.insert(k, px);
    }
    pub fn tiles(&self) -> usize {
        self.saved.len()
    }
    /// The undo record: one change per saved tile.
    pub fn finish(self, img: &Image) -> Vec<Change> {
        let mut keys: Vec<_> = self.saved.keys().copied().collect();
        keys.sort_unstable();
        let mut saved = self.saved;
        keys.into_iter()
            .filter_map(|k| {
                let px = saved.remove(&k)?;
                let (x0, y0) = (k.0 * TILE, k.1 * TILE);
                Some(Change::Image { image: self.image, x: x0, y: y0, w: TILE.min(img.width - x0), h: TILE.min(img.height - y0), pixels: px })
            })
            .collect()
    }
}

/// Bilinear sample at texel coordinates (pixel centres at +0.5), clamped.
fn sample_px(img: &Image, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (img.width as i64, img.height as i64);
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |xi: i64, yi: i64| img.pixels[(yi.clamp(0, h - 1) * w + xi.clamp(0, w - 1)) as usize];
    let (x0, y0) = (x0 as i64, y0 as i64);
    let a = mix4(at(x0, y0), at(x0 + 1, y0), tx);
    let b = mix4(at(x0, y0 + 1), at(x0 + 1, y0 + 1), tx);
    mix4(a, b, ty)
}

fn uv_to_px(img: &Image, uv: Vec2) -> (f32, f32) {
    (uv.x * img.width as f32, (1.0 - uv.y) * img.height as f32)
}

/// A running texture paint stroke on one image.
pub struct TexStroke {
    pub undo: TileUndo,
    stamp: Vec<u32>,
    gen: u32,
    /// Texels covered by some UV triangle (3D painting, for seam bleed).
    coverage: Vec<bool>,
    /// Clone: offset from the painted texel to its source, in texels.
    pub clone_offset: (f32, f32),
    pub dabs: usize,
}

/// One written texel.
struct Write {
    i: usize,
    c: [f32; 4],
}

impl TexStroke {
    pub fn new(image: usize, img: &Image) -> TexStroke {
        TexStroke { undo: TileUndo::new(image), stamp: vec![0; (img.width * img.height) as usize], gen: 0, coverage: Vec::new(), clone_offset: (0.0, 0.0), dabs: 0 }
    }

    fn next_gen(&mut self) -> u32 {
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.gen = 1;
        }
        self.gen
    }

    /// The brush's new color for texel `i` at strength `f` (None: no change).
    #[allow(clippy::too_many_arguments)]
    fn texel(&self, img: &Image, s: &TpSettings, i: usize, f: f32, invert: bool, smear_src: Option<(f32, f32)>, mask: &mut Option<&mut Vec<f32>>) -> Option<[f32; 4]> {
        let w = img.width as usize;
        let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
        let mut f = f.clamp(0.0, 1.0);
        if s.brush == TexBrush::Mask {
            if let Some(m) = mask.as_deref_mut() {
                if m.len() != img.pixels.len() {
                    m.resize(img.pixels.len(), 0.0);
                }
                m[i] = (m[i] + if invert { -f } else { f }).clamp(0.0, 1.0);
            }
            return None;
        }
        if s.use_mask {
            if let Some(m) = mask.as_deref() {
                f *= 1.0 - m.get(i).copied().unwrap_or(0.0);
            }
        }
        if f <= 0.0 {
            return None;
        }
        let cur = img.pixels[i];
        Some(match s.brush {
            TexBrush::Draw | TexBrush::Fill => blend(s.blend, cur, if invert { s.secondary } else { s.color }, f),
            TexBrush::Soften => {
                let mut acc = [0.0; 4];
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let c = sample_px(img, x + dx as f32, y + dy as f32);
                        for k in 0..4 {
                            acc[k] += c[k] / 9.0;
                        }
                    }
                }
                // Ctrl sharpens instead.
                if invert {
                    let sharp = std::array::from_fn(|k| (cur[k] * 2.0 - acc[k]).max(0.0));
                    mix4(cur, sharp, f)
                } else {
                    mix4(cur, acc, f)
                }
            }
            TexBrush::Smear => {
                let (sx, sy) = smear_src?;
                mix4(cur, sample_px(img, sx, sy), f)
            }
            TexBrush::Clone => {
                let src = sample_px(img, x + self.clone_offset.0, y + self.clone_offset.1);
                blend(s.blend, cur, [src[0], src[1], src[2]], f * src[3])
            }
            TexBrush::Mask => return None,
        })
    }

    fn commit(&mut self, img: &mut Image, writes: Vec<Write>) -> usize {
        let w = img.width;
        for wr in &writes {
            self.undo.save(img, wr.i as u32 % w, wr.i as u32 / w);
        }
        let n = writes.len();
        for wr in writes {
            img.pixels[wr.i] = wr.c;
        }
        n
    }

    /// One dab in the Image Editor: `at` and `radius` in texels, `step`
    /// the pointer movement in texels (Smear). Returns texels written.
    #[allow(clippy::too_many_arguments)]
    pub fn dab_2d(&mut self, img: &mut Image, s: &TpSettings, at: (f32, f32), radius: f32, pressure: f32, step: (f32, f32), invert: bool, mut mask: Option<&mut Vec<f32>>) -> usize {
        if img.width == 0 || img.height == 0 || img.pixels.len() != (img.width * img.height) as usize {
            return 0;
        }
        self.dabs += 1;
        if s.brush == TexBrush::Fill {
            return self.fill_2d(img, s, at, invert);
        }
        let r = radius.max(0.5);
        let (x0, x1) = (((at.0 - r).floor().max(0.0)) as u32, ((at.0 + r).ceil().min(img.width as f32)) as u32);
        let (y0, y1) = (((at.1 - r).floor().max(0.0)) as u32, ((at.1 + r).ceil().min(img.height as f32)) as u32);
        let mut writes = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let d = ((cx - at.0).powi(2) + (cy - at.1).powi(2)).sqrt() / r;
                if d >= 1.0 {
                    continue;
                }
                let f = s.curve.eval(d) * s.strength * pressure;
                let i = (y * img.width + x) as usize;
                let src = Some((cx - step.0, cy - step.1));
                if let Some(c) = self.texel(img, s, i, f, invert, src, &mut mask) {
                    writes.push(Write { i, c });
                }
            }
        }
        self.commit(img, writes)
    }

    /// Bucket fill from texel `at` over pixels within the threshold.
    fn fill_2d(&mut self, img: &mut Image, s: &TpSettings, at: (f32, f32), invert: bool) -> usize {
        let (w, h) = (img.width as i64, img.height as i64);
        let (sx, sy) = (at.0 as i64, at.1 as i64);
        if sx < 0 || sy < 0 || sx >= w || sy >= h {
            return 0;
        }
        let seed = img.pixels[(sy * w + sx) as usize];
        let gen = self.next_gen();
        let mut stack = vec![(sx, sy)];
        let mut writes = Vec::new();
        let close = |c: [f32; 4]| (0..4).map(|k| (c[k] - seed[k]).abs()).fold(0.0f32, f32::max) <= s.fill_threshold;
        while let Some((x, y)) = stack.pop() {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let i = (y * w + x) as usize;
            if self.stamp[i] == gen || !close(img.pixels[i]) {
                continue;
            }
            self.stamp[i] = gen;
            writes.push(Write { i, c: blend(s.blend, img.pixels[i], if invert { s.secondary } else { s.color }, s.strength) });
            stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
        self.commit(img, writes)
    }

    /// Texels inside some UV triangle of `m`.
    fn ensure_coverage(&mut self, img: &Image, m: &Mesh) {
        if self.coverage.len() == img.pixels.len() {
            return;
        }
        let mut cov = vec![false; img.pixels.len()];
        for t in m.triangulate() {
            let f = &m.faces[t.face as usize];
            if f.uvs.len() != f.verts.len() {
                continue;
            }
            let uv = t.corners.map(|c| uv_to_px(img, f.uvs[c as usize]));
            raster_uv(img, uv, 0.0, |i, _| cov[i] = true);
        }
        self.coverage = cov;
    }

    /// One dab from the 3D view: paints texels of `img` whose surface point
    /// projects within `radius` pixels of `at`. `project` maps object space
    /// to (screen x, screen y, depth: smaller is nearer). Returns the number
    /// of texels written (bleed included).
    #[allow(clippy::too_many_arguments)]
    pub fn dab_3d(&mut self, img: &mut Image, m: &Mesh, project: &dyn Fn(Vec3) -> Option<(f32, f32, f32)>, s: &TpSettings, at: (f32, f32), radius: f32, pressure: f32, step: (f32, f32), invert: bool, mut mask: Option<&mut Vec<f32>>) -> usize {
        if img.width == 0 || img.height == 0 || img.pixels.len() != (img.width * img.height) as usize {
            return 0;
        }
        self.dabs += 1;
        self.ensure_coverage(img, m);
        let r = radius.max(0.5);
        let scr: Vec<Option<(f32, f32, f32)>> = m.positions.iter().map(|&p| project(p)).collect();
        let tris = m.triangulate();
        let fill = s.brush == TexBrush::Fill;
        let front = |a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32)| (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0) < 0.0;
        // Depth buffer over the brush (occlusion).
        let (bx0, by0) = ((at.0 - r).floor() as i32, (at.1 - r).floor() as i32);
        let bw = (2.0 * r).ceil() as i32 + 2;
        let mut zb = vec![f32::INFINITY; (bw * bw) as usize];
        if s.occlude && !fill {
            for t in &tris {
                let (Some(a), Some(b), Some(c)) = (scr[t.verts[0] as usize], scr[t.verts[1] as usize], scr[t.verts[2] as usize]) else { continue };
                raster_screen([a, b, c], bx0, by0, bw, |k, z| {
                    if z < zb[k] {
                        zb[k] = z;
                    }
                });
            }
        }
        let gen = self.next_gen();
        let mut writes: Vec<Write> = Vec::new();
        let w = img.width as usize;
        for t in &tris {
            let f = &m.faces[t.face as usize];
            if f.uvs.len() != f.verts.len() || m.is_face_hidden(t.face as usize) {
                continue;
            }
            let (Some(a), Some(b), Some(c)) = (scr[t.verts[0] as usize], scr[t.verts[1] as usize], scr[t.verts[2] as usize]) else { continue };
            if s.backface_culling && !front(a, b, c) {
                continue;
            }
            if !fill {
                let (minx, maxx) = (a.0.min(b.0).min(c.0), a.0.max(b.0).max(c.0));
                let (miny, maxy) = (a.1.min(b.1).min(c.1), a.1.max(b.1).max(c.1));
                if maxx < at.0 - r || minx > at.0 + r || maxy < at.1 - r || miny > at.1 + r {
                    continue;
                }
            }
            let uvp = t.corners.map(|k| uv_to_px(img, f.uvs[k as usize]));
            // Screen movement to texel movement on this triangle (Smear).
            let jac = {
                let (s1, s2) = ((b.0 - a.0, b.1 - a.1), (c.0 - a.0, c.1 - a.1));
                let (u1, u2) = ((uvp[1].0 - uvp[0].0, uvp[1].1 - uvp[0].1), (uvp[2].0 - uvp[0].0, uvp[2].1 - uvp[0].1));
                let det = s1.0 * s2.1 - s2.0 * s1.1;
                if det.abs() < 1e-12 {
                    None
                } else {
                    // J = U * S^-1
                    let inv = [[s2.1 / det, -s2.0 / det], [-s1.1 / det, s1.0 / det]];
                    Some([[u1.0 * inv[0][0] + u2.0 * inv[1][0], u1.0 * inv[0][1] + u2.0 * inv[1][1]], [u1.1 * inv[0][0] + u2.1 * inv[1][0], u1.1 * inv[0][1] + u2.1 * inv[1][1]]])
                }
            };
            let mut list: Vec<(usize, [f32; 3])> = Vec::new();
            raster_uv(img, uvp, 0.0, |i, bary| list.push((i, bary)));
            for (i, bary) in list {
                if self.stamp[i] == gen {
                    continue;
                }
                let sx = a.0 * bary[0] + b.0 * bary[1] + c.0 * bary[2];
                let sy = a.1 * bary[0] + b.1 * bary[1] + c.1 * bary[2];
                let sz = a.2 * bary[0] + b.2 * bary[1] + c.2 * bary[2];
                let fval = if fill {
                    s.strength
                } else {
                    let d = ((sx - at.0).powi(2) + (sy - at.1).powi(2)).sqrt() / r;
                    if d >= 1.0 {
                        continue;
                    }
                    if s.occlude {
                        let (zx, zy) = (sx.floor() as i32 - bx0, sy.floor() as i32 - by0);
                        if zx >= 0 && zy >= 0 && zx < bw && zy < bw {
                            let z = zb[(zy * bw + zx) as usize];
                            if z.is_finite() && sz > z + 1e-3 * z.abs().max(1.0) {
                                continue;
                            }
                        }
                    }
                    s.curve.eval(d) * s.strength * pressure
                };
                self.stamp[i] = gen;
                let (x, y) = ((i % w) as f32 + 0.5, (i / w) as f32 + 0.5);
                let src = jac.map(|j| (x - (j[0][0] * step.0 + j[0][1] * step.1), y - (j[1][0] * step.0 + j[1][1] * step.1)));
                if let Some(c) = self.texel(img, s, i, fval, invert, src, &mut mask) {
                    writes.push(Write { i, c });
                }
            }
        }
        // Seam bleed: spread written texels into uncovered neighbours.
        if s.bleed > 0 && !writes.is_empty() {
            let b = s.bleed as i64;
            let (iw, ih) = (img.width as i64, img.height as i64);
            let mut extra = Vec::new();
            for wr in &writes {
                let (x, y) = ((wr.i % w) as i64, (wr.i / w) as i64);
                for dy in -b..=b {
                    for dx in -b..=b {
                        let (xx, yy) = (x + dx, y + dy);
                        if xx < 0 || yy < 0 || xx >= iw || yy >= ih {
                            continue;
                        }
                        let j = (yy * iw + xx) as usize;
                        if self.coverage[j] || self.stamp[j] == gen {
                            continue;
                        }
                        self.stamp[j] = gen;
                        extra.push(Write { i: j, c: wr.c });
                    }
                }
            }
            writes.extend(extra);
        }
        self.commit(img, writes)
    }
}

/// Visit texels whose centres lie in a triangle given in texel coordinates
/// (`grow` widens it), with barycentrics.
fn raster_uv(img: &Image, p: [(f32, f32); 3], grow: f32, mut f: impl FnMut(usize, [f32; 3])) {
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
    if area.abs() < 1e-12 {
        return;
    }
    let x0 = (p[0].0.min(p[1].0).min(p[2].0) - grow).floor().max(0.0) as i64;
    let x1 = (p[0].0.max(p[1].0).max(p[2].0) + grow).ceil().min(img.width as f32) as i64;
    let y0 = (p[0].1.min(p[1].1).min(p[2].1) - grow).floor().max(0.0) as i64;
    let y1 = (p[0].1.max(p[1].1).max(p[2].1) + grow).ceil().min(img.height as f32) as i64;
    // Edge lengths for the widening test.
    let len = |a: (f32, f32), b: (f32, f32)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1e-9);
    let l = [len(p[1], p[2]), len(p[2], p[0]), len(p[0], p[1])];
    for y in y0..y1 {
        for x in x0..x1 {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].0 - cx) * (p[2].1 - cy) - (p[1].1 - cy) * (p[2].0 - cx)) / area;
            let w1 = ((p[2].0 - cx) * (p[0].1 - cy) - (p[2].1 - cy) * (p[0].0 - cx)) / area;
            let w2 = 1.0 - w0 - w1;
            // Signed distances to the edges in texels.
            let ok = [w0, w1, w2].iter().zip(&l).all(|(w, l)| w * area.abs() / l >= -grow - 1e-5);
            if ok {
                f((y * img.width as i64 + x) as usize, [w0, w1, w2]);
            }
        }
    }
}

/// Rasterize a screen triangle's depth into a `bw` x `bw` buffer at (bx0, by0).
fn raster_screen(p: [(f32, f32, f32); 3], bx0: i32, by0: i32, bw: i32, mut f: impl FnMut(usize, f32)) {
    let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
    if area.abs() < 1e-12 {
        return;
    }
    let x0 = (p[0].0.min(p[1].0).min(p[2].0).floor() as i32).max(bx0);
    let x1 = (p[0].0.max(p[1].0).max(p[2].0).ceil() as i32).min(bx0 + bw - 1);
    let y0 = (p[0].1.min(p[1].1).min(p[2].1).floor() as i32).max(by0);
    let y1 = (p[0].1.max(p[1].1).max(p[2].1).ceil() as i32).min(by0 + bw - 1);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].0 - cx) * (p[2].1 - cy) - (p[1].1 - cy) * (p[2].0 - cx)) / area;
            let w1 = ((p[2].0 - cx) * (p[0].1 - cy) - (p[2].1 - cy) * (p[0].0 - cx)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            f(((y - by0) * bw + (x - bx0)) as usize, p[0].2 * w0 + p[1].2 * w1 + p[2].2 * w2);
        }
    }
}

/// The texel (x, y) under a surface point given by face `face` and the
/// point `p` on it (object space): UVs interpolated over the triangle.
pub fn uv_at(m: &Mesh, face: usize, p: Vec3) -> Option<Vec2> {
    let f = m.faces.get(face)?;
    if f.uvs.len() != f.verts.len() {
        return None;
    }
    let mut best: Option<(f32, Vec2)> = None;
    for t in m.triangulate().into_iter().filter(|t| t.face as usize == face) {
        let [a, b, c] = t.verts.map(|v| m.positions[v as usize]);
        let (e1, e2, d) = (b - a, c - a, p - a);
        let (d00, d01, d11, d20, d21) = (e1.dot(e1), e1.dot(e2), e2.dot(e2), d.dot(e1), d.dot(e2));
        let den = d00 * d11 - d01 * d01;
        if den.abs() < 1e-20 {
            continue;
        }
        let v = (d11 * d20 - d01 * d21) / den;
        let w = (d00 * d21 - d01 * d20) / den;
        let u = 1.0 - v - w;
        let err = (-u).max(-v).max(-w).max(0.0);
        let uv = f.uvs[t.corners[0] as usize] * u + f.uvs[t.corners[1] as usize] * v + f.uvs[t.corners[2] as usize] * w;
        if best.is_none_or(|b| err < b.0) {
            best = Some((err, uv));
        }
    }
    best.map(|b| b.1)
}
