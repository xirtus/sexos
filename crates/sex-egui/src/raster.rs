// SPDX-License-Identifier: GPL-3.0-only
//! A CPU rasterizer for egui's meshes, straight into a SexOS window's pixels.
//!
//! egui draws everything as textured triangles with premultiplied colours, and
//! anti-aliases by feathering edges with transparent vertices, so a rasterizer needs
//! no coverage sampling of its own: one sample at each pixel centre, the vertex colour
//! and texture interpolated across the triangle, then "source over" onto the window.
//! Triangles share edges, so the top-left rule decides which one owns a pixel on an
//! edge and no pixel is blended twice.
//!
//! Window pixels are `0x00RRGGBB` words. Blending happens in sRGB (gamma) space, as
//! egui's own software and web painters do.
use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, ImageDelta, Primitive, TextureId, Vertex};
use egui::{Color32, TextureFilter, TexturesDelta};

/// A texture egui uploaded: premultiplied RGBA texels.
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub texels: Vec<[u8; 4]>,
    pub linear: bool,
}

impl Texture {
    #[inline]
    fn texel(&self, x: i32, y: i32) -> [u8; 4] {
        let x = x.clamp(0, self.width as i32 - 1) as usize;
        let y = y.clamp(0, self.height as i32 - 1) as usize;
        self.texels[y * self.width + x]
    }
    /// The texture at `u`, `v` (0..1), as premultiplied RGBA in 0..255.
    #[inline]
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let fx = u * self.width as f32 - 0.5;
        let fy = v * self.height as f32 - 0.5;
        if !self.linear {
            let t = self.texel((fx + 0.5).floor() as i32, (fy + 0.5).floor() as i32);
            return [t[0] as f32, t[1] as f32, t[2] as f32, t[3] as f32];
        }
        let x0 = fx.floor();
        let y0 = fy.floor();
        let (ax, ay) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let t00 = self.texel(x0, y0);
        let t10 = self.texel(x0 + 1, y0);
        let t01 = self.texel(x0, y0 + 1);
        let t11 = self.texel(x0 + 1, y0 + 1);
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = t00[i] as f32 + (t10[i] as f32 - t00[i] as f32) * ax;
            let bottom = t01[i] as f32 + (t11[i] as f32 - t01[i] as f32) * ax;
            out[i] = top + (bottom - top) * ay;
        }
        out
    }
}

/// Every texture egui has uploaded and not yet freed.
#[derive(Default)]
pub struct Textures {
    map: HashMap<TextureId, Texture>,
}

impl Textures {
    pub fn new() -> Self { Self::default() }
    pub fn get(&self, id: TextureId) -> Option<&Texture> { self.map.get(&id) }

    /// New and changed textures, before painting a frame.
    pub fn set(&mut self, delta: &TexturesDelta) {
        for (id, deltas) in &delta.set {
            for d in deltas { self.apply(*id, d); }
        }
    }
    /// Textures egui no longer needs, after painting the frame. The delta is used up.
    pub fn free(&mut self, delta: &mut TexturesDelta) {
        for id in &delta.free { self.map.remove(id); }
        delta.clear();
    }

    fn apply(&mut self, id: TextureId, delta: &ImageDelta) {
        let egui::ImageData::Color(image) = &delta.image;
        let [w, h] = image.size;
        let texels = image.pixels.iter().map(|c: &Color32| c.to_array());
        let linear = delta.options.magnification == TextureFilter::Linear;
        match delta.pos {
            None => {
                self.map.insert(id, Texture { width: w, height: h, texels: texels.collect(), linear });
            }
            Some([px, py]) => {
                let Some(t) = self.map.get_mut(&id) else { return };
                for (i, texel) in texels.enumerate() {
                    let (x, y) = (px + i % w, py + i / w);
                    if x < t.width && y < t.height { t.texels[y * t.width + x] = texel; }
                }
            }
        }
    }
}

/// A window's pixel buffer: `width` by `height` visible pixels, `stride` words a row.
pub struct Target<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    pub stride: usize,
}

impl Target<'_> {
    pub fn clear(&mut self, color: Color32) {
        let word = word(color);
        for y in 0..self.height {
            self.pixels[y * self.stride..y * self.stride + self.width].fill(word);
        }
    }
}

#[inline]
fn word(c: Color32) -> u32 { (c.r() as u32) << 16 | (c.g() as u32) << 8 | c.b() as u32 }

/// Counts for one painted frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats { pub meshes: usize, pub triangles: usize, pub pixels: usize }

/// Paints egui's tessellated primitives, at `ppp` pixels per point, in order.
pub fn paint(target: &mut Target, primitives: &[ClippedPrimitive], textures: &Textures, ppp: f32) -> Stats {
    let mut stats = Stats::default();
    for p in primitives {
        let Primitive::Mesh(mesh) = &p.primitive else { continue };
        let Some(texture) = textures.get(mesh.texture_id) else { continue };
        // The clip rectangle in whole pixels, inside the target.
        let clip = Clip {
            x0: ((p.clip_rect.min.x * ppp).round().max(0.0) as i32),
            y0: ((p.clip_rect.min.y * ppp).round().max(0.0) as i32),
            x1: ((p.clip_rect.max.x * ppp).round().min(target.width as f32) as i32),
            y1: ((p.clip_rect.max.y * ppp).round().min(target.height as f32) as i32),
        };
        if clip.x0 >= clip.x1 || clip.y0 >= clip.y1 { continue; }
        stats.meshes += 1;
        for tri in mesh.indices.chunks_exact(3) {
            let v = |i: u32| &mesh.vertices[i as usize];
            stats.pixels += triangle(target, &clip, texture, ppp, [v(tri[0]), v(tri[1]), v(tri[2])]);
            stats.triangles += 1;
        }
    }
    stats
}

struct Clip { x0: i32, y0: i32, x1: i32, y1: i32 }

#[inline]
fn rgba(c: Color32) -> [f32; 4] { [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32] }

/// Rasterizes one triangle; returns how many pixels it touched.
fn triangle(target: &mut Target, clip: &Clip, texture: &Texture, ppp: f32, v: [&Vertex; 3]) -> usize {
    let p = [v[0].pos * ppp, v[1].pos * ppp, v[2].pos * ppp];
    let mut area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[1].y - p[0].y) * (p[2].x - p[0].x);
    if area.abs() < 1e-6 { return 0; }
    // Wind every triangle the same way so the edge functions are positive inside.
    let order = if area < 0.0 { area = -area; [0, 2, 1] } else { [0, 1, 2] };
    let (a, b, c) = (p[order[0]], p[order[1]], p[order[2]]);
    let (va, vb, vc) = (v[order[0]], v[order[1]], v[order[2]]);

    let x0 = (a.x.min(b.x).min(c.x).floor() as i32).max(clip.x0);
    let x1 = (a.x.max(b.x).max(c.x).ceil() as i32).min(clip.x1);
    let y0 = (a.y.min(b.y).min(c.y).floor() as i32).max(clip.y0);
    let y1 = (a.y.max(b.y).max(c.y).ceil() as i32).min(clip.y1);
    if x0 >= x1 || y0 >= y1 { return 0; }

    // Edge i is opposite vertex i: w0 weights a, w1 weights b, w2 weights c.
    let edge = |p: egui::Pos2, q: egui::Pos2| (q.y - p.y, p.x - q.x, p.x * q.y - p.y * q.x);
    let edges = [edge(b, c), edge(c, a), edge(a, b)];
    // Top-left rule: a pixel exactly on an edge belongs to this triangle only if the
    // edge is a top edge or a left edge.
    let top_left = |(dy, dx, _): (f32, f32, f32)| (dy == 0.0 && dx < 0.0) || dy > 0.0;
    let bias: [f32; 3] = core::array::from_fn(|i| if top_left(edges[i]) { 0.0 } else { -1e-5 });

    let flat_uv = va.uv == vb.uv && vb.uv == vc.uv;
    let flat_color = va.color == vb.color && vb.color == vc.color;
    let (ca, cb, cc) = (rgba(va.color), rgba(vb.color), rgba(vc.color));
    // A solid colour when the texture is one texel (egui's white pixel for shapes).
    let flat = flat_uv.then(|| {
        let t = texture.sample(va.uv.x, va.uv.y);
        let mut out = [0.0; 4];
        for i in 0..4 { out[i] = ca[i] * t[i] / 255.0; }
        out
    });

    let inv = 1.0 / area;
    let mut touched = 0;
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let row = y as usize * target.stride;
        for x in x0..x1 {
            let px = x as f32 + 0.5;
            let w: [f32; 3] = core::array::from_fn(|i| edges[i].0 * px + edges[i].1 * py - edges[i].2);
            // Signs: with this winding the edge functions are negative inside.
            if -w[0] + bias[0] < 0.0 || -w[1] + bias[1] < 0.0 || -w[2] + bias[2] < 0.0 { continue; }
            let (l0, l1, l2) = (-w[0] * inv, -w[1] * inv, -w[2] * inv);
            let src = match (flat, flat_color) {
                (Some(s), true) => s,
                _ => {
                    let col: [f32; 4] = core::array::from_fn(|i| ca[i] * l0 + cb[i] * l1 + cc[i] * l2);
                    let t = if flat_uv {
                        texture.sample(va.uv.x, va.uv.y)
                    } else {
                        let u = va.uv.x * l0 + vb.uv.x * l1 + vc.uv.x * l2;
                        let v = va.uv.y * l0 + vb.uv.y * l1 + vc.uv.y * l2;
                        texture.sample(u, v)
                    };
                    core::array::from_fn(|i| col[i] * t[i] / 255.0)
                }
            };
            let alpha = src[3];
            if alpha <= 0.0 && src[0] <= 0.0 && src[1] <= 0.0 && src[2] <= 0.0 { continue; }
            let i = row + x as usize;
            let dst = target.pixels[i];
            let keep = 1.0 - alpha / 255.0;
            let blend = |s: f32, d: u32| (s + d as f32 * keep + 0.5).clamp(0.0, 255.0) as u32;
            target.pixels[i] = blend(src[0], dst >> 16 & 0xFF) << 16 | blend(src[1], dst >> 8 & 0xFF) << 8 | blend(src[2], dst & 0xFF);
            touched += 1;
        }
    }
    touched
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::{Mesh, WHITE_UV};
    use egui::{Pos2, Rect};

    fn white() -> Textures {
        let mut t = Textures::new();
        t.map.insert(TextureId::default(), Texture { width: 1, height: 1, texels: vec![[255; 4]], linear: false });
        t
    }
    fn quad(rect: Rect, color: Color32) -> ClippedPrimitive {
        let mut mesh = Mesh::default();
        mesh.add_colored_rect(rect, color);
        ClippedPrimitive { clip_rect: Rect::EVERYTHING, primitive: Primitive::Mesh(mesh) }
    }

    #[test]
    fn an_opaque_rectangle_fills_exactly_its_pixels() {
        let mut px = vec![0u32; 16 * 16];
        let mut target = Target { pixels: &mut px, width: 16, height: 16, stride: 16 };
        let stats = paint(&mut target, &[quad(Rect::from_min_max(Pos2::new(2.0, 3.0), Pos2::new(10.0, 7.0)), Color32::RED)], &white(), 1.0);
        // Two triangles share the diagonal; the top-left rule blends no pixel twice.
        assert_eq!(stats.pixels, 8 * 4);
        assert_eq!(px[3 * 16 + 2], 0xFF0000);
        assert_eq!(px[6 * 16 + 9], 0xFF0000);
        assert_eq!(px[7 * 16 + 9], 0);
        assert_eq!(px[3 * 16 + 10], 0);
    }

    #[test]
    fn half_transparent_premultiplied_blends_over() {
        let mut px = vec![0x0000FFu32; 4 * 4];
        let mut target = Target { pixels: &mut px, width: 4, height: 4, stride: 4 };
        let half_red = Color32::from_rgba_premultiplied(128, 0, 0, 128);
        paint(&mut target, &[quad(Rect::from_min_max(Pos2::ZERO, Pos2::new(4.0, 4.0)), half_red)], &white(), 1.0);
        assert_eq!(px[5], 128 << 16 | 127);
    }

    #[test]
    fn clipping_and_scale_apply() {
        let mut px = vec![0u32; 20 * 20];
        let mut target = Target { pixels: &mut px, width: 20, height: 20, stride: 20 };
        let mut p = quad(Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 10.0)), Color32::WHITE);
        p.clip_rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(5.0, 5.0));
        let stats = paint(&mut target, &[p], &white(), 2.0);
        assert_eq!(stats.pixels, 10 * 10);
        assert_eq!(px[9 * 20 + 9], 0xFFFFFF);
        assert_eq!(px[10 * 20 + 10], 0);
    }

    #[test]
    fn textures_are_sampled_and_updated_in_part() {
        let mut textures = Textures::new();
        let mut delta = TexturesDelta::default();
        let image = egui::ColorImage::new([2, 2], vec![Color32::RED, Color32::GREEN, Color32::BLUE, Color32::WHITE]);
        delta.set.insert(TextureId::Managed(7), [ImageDelta::full(image, egui::TextureOptions::NEAREST)].into_iter().collect());
        textures.set(&delta);
        textures.free(&mut delta);
        let t = textures.get(TextureId::Managed(7)).unwrap();
        assert_eq!(t.sample(0.25, 0.25), [255.0, 0.0, 0.0, 255.0]);
        assert_eq!(t.sample(0.75, 0.75), [255.0, 255.0, 255.0, 255.0]);
        let mut patch = TexturesDelta::default();
        let one = egui::ColorImage::new([1, 1], vec![Color32::BLACK]);
        patch.set.insert(TextureId::Managed(7), [ImageDelta::partial([1, 1], one, egui::TextureOptions::NEAREST)].into_iter().collect());
        textures.set(&patch);
        textures.free(&mut patch);
        assert_eq!(textures.get(TextureId::Managed(7)).unwrap().sample(0.75, 0.75), [0.0, 0.0, 0.0, 255.0]);
        let _ = WHITE_UV;
    }
}
