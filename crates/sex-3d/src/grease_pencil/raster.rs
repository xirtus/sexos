// SPDX-License-Identifier: GPL-3.0-only
//! Drawing Grease Pencil: strokes as screen-aligned ribbons with round (or
//! flat) caps, dots and squares, fills triangulated by ear clipping, all
//! anti-aliased, depth tested against the scene's depth (distance along the
//! view axis) and composited per layer (blend mode, opacity, tint), then per
//! object (visual effects), objects back to front over the image.
use super::effects::{apply_effects, GpEffect};
use super::geom::{self, seg_dist2};
use super::modifiers::{evaluate, EvalContext, EvalLayer};
use super::view::GpView;
use super::{CapType, FillStyle, GpMaterial, GpStroke, GradientType, GreasePencil, LayerBlend, LineMode, OnionMode, StrokeStyle};
use crate::math::{Mat4, Vec2};
use crate::scene::{Image, ObjectData, Scene};

/// A premultiplied linear RGBA canvas with the bounds of what was drawn.
#[derive(Clone, Debug, Default)]
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
    /// Drawn region (x0, y0, x1, y1), exclusive; empty when x0 >= x1.
    pub bounds: (usize, usize, usize, usize),
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Canvas {
        Canvas { w, h, px: vec![[0.0; 4]; w * h], bounds: (w, h, 0, 0) }
    }
    pub fn clear(&mut self) {
        let (x0, y0, x1, y1) = self.bounds;
        for y in y0..y1.min(self.h) {
            self.px[y * self.w + x0..y * self.w + x1.min(self.w)].fill([0.0; 4]);
        }
        self.bounds = (self.w, self.h, 0, 0);
    }
    pub fn is_empty(&self) -> bool {
        self.bounds.0 >= self.bounds.2 || self.bounds.1 >= self.bounds.3
    }
    fn grow(&mut self, x: usize, y: usize) {
        let b = &mut self.bounds;
        b.0 = b.0.min(x);
        b.1 = b.1.min(y);
        b.2 = b.2.max(x + 1);
        b.3 = b.3.max(y + 1);
    }
    pub fn mark_all(&mut self) {
        self.bounds = (0, 0, self.w, self.h);
    }
    /// Source-over a premultiplied colour.
    fn over(&mut self, x: usize, y: usize, c: [f32; 4]) {
        let i = y * self.w + x;
        let d = &mut self.px[i];
        let k = 1.0 - c[3];
        for j in 0..4 {
            d[j] = c[j] + d[j] * k;
        }
        self.grow(x, y);
    }
    /// Erase by coverage (holdout).
    fn cut(&mut self, x: usize, y: usize, cov: f32) {
        let d = &mut self.px[y * self.w + x];
        for v in d.iter_mut() {
            *v *= 1.0 - cov;
        }
    }
}

/// Everything a stroke needs to resolve its look.
struct Look<'a> {
    materials: &'a [GpMaterial],
    images: &'a [Image],
    tint: ([f32; 3], f32),
    /// Onion skin: replace colours with this (alpha factor in .3).
    ghost: Option<[f32; 4]>,
}

/// Per-stroke scratch: coverage, colour and depth of the strongest fragment.
#[derive(Default)]
struct Scratch {
    x0: i32,
    y0: i32,
    w: usize,
    h: usize,
    cov: Vec<f32>,
    col: Vec<[f32; 4]>,
    depth: Vec<f32>,
}

impl Scratch {
    fn reset(&mut self, x0: i32, y0: i32, w: usize, h: usize) {
        (self.x0, self.y0, self.w, self.h) = (x0, y0, w, h);
        self.cov.clear();
        self.cov.resize(w * h, 0.0);
        self.col.resize(w * h, [0.0; 4]);
        self.depth.resize(w * h, 0.0);
    }
    fn put(&mut self, x: i32, y: i32, cov: f32, col: [f32; 4], depth: f32) {
        let (lx, ly) = (x - self.x0, y - self.y0);
        if lx < 0 || ly < 0 || lx as usize >= self.w || ly as usize >= self.h {
            return;
        }
        let i = ly as usize * self.w + lx as usize;
        if cov > self.cov[i] {
            self.cov[i] = cov;
            self.col[i] = col;
            self.depth[i] = depth;
        }
    }
}

/// A projected point.
#[derive(Clone, Copy)]
struct SP {
    x: f32,
    y: f32,
    dist: f32,
    r: f32,
    col: [f32; 4],
    u: f32,
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t, a[3] + (b[3] - a[3]) * t]
}

/// Drawing context of one layer.
pub struct LayerTarget<'a> {
    pub view: &'a GpView,
    /// Scene depth (distance along the view axis), `w * h`, or none.
    pub depth: Option<&'a [f32]>,
    /// Layer to world.
    pub world: Mat4,
}

fn depth_ok(t: &LayerTarget<'_>, w: usize, x: usize, y: usize, d: f32) -> bool {
    match t.depth {
        Some(z) => {
            let s = z[y * w + x];
            d <= s + (s.abs() * 1e-3).max(1e-4)
        }
        None => true,
    }
}

/// Draw one stroke into `canvas`; `cut` collects holdout coverage.
fn draw_stroke(s: &GpStroke, look: &Look<'_>, t: &LayerTarget<'_>, radius_offset: f32, canvas: &mut Canvas, cut: &mut Canvas, sc: &mut Scratch) {
    let Some(mat) = look.materials.get(s.material).or(look.materials.first()) else { return };
    if mat.hide || s.points.is_empty() {
        return;
    }
    let scale = super::scale_of(&t.world);
    let lens = geom::arc_lengths(&s.points);
    let total = lens.last().copied().unwrap_or(0.0).max(1e-9);
    let mut pts: Vec<SP> = Vec::with_capacity(s.points.len());
    for (i, p) in s.points.iter().enumerate() {
        let w = t.world.transform_point(p.position);
        let Some((x, y, dist)) = t.view.project(w) else { continue };
        let r = ((p.radius + radius_offset).max(0.0) * scale * t.view.pixels_per_unit(dist)).max(0.0);
        let base = mat.color;
        let vc = p.vertex_color;
        let mut rgb = mix3([base[0], base[1], base[2]], [vc[0], vc[1], vc[2]], vc[3].clamp(0.0, 1.0));
        rgb = mix3(rgb, look.tint.0, look.tint.1);
        let mut a = base[3] * p.opacity.clamp(0.0, 1.0);
        if let Some(g) = look.ghost {
            rgb = [g[0], g[1], g[2]];
            a *= g[3];
        }
        pts.push(SP { x, y, dist, r, col: [rgb[0] * a, rgb[1] * a, rgb[2] * a, a], u: lens[i] / total });
    }
    if pts.is_empty() {
        return;
    }
    let (w, h) = (canvas.w as i32, canvas.h as i32);
    // Fill first, under the stroke line.
    if mat.show_fill && s.points.len() >= 3 {
        draw_fill(s, mat, look, t, &pts, canvas, cut);
    }
    if !mat.show_stroke {
        return;
    }
    let rmax = pts.iter().map(|p| p.r).fold(0.0f32, f32::max) + 2.0;
    let x0 = (pts.iter().map(|p| p.x).fold(f32::INFINITY, f32::min) - rmax).floor().max(0.0) as i32;
    let y0 = (pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min) - rmax).floor().max(0.0) as i32;
    let x1 = ((pts.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max) + rmax).ceil() as i32).min(w - 1);
    let y1 = ((pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max) + rmax).ceil() as i32).min(h - 1);
    if x0 > x1 || y0 > y1 {
        return;
    }
    sc.reset(x0, y0, (x1 - x0 + 1) as usize, (y1 - y0 + 1) as usize);
    let soft = s.softness.clamp(0.0, 0.999);
    let tex = if mat.stroke_style == StrokeStyle::Texture { mat.stroke_image.and_then(|i| look.images.get(i)) } else { None };
    let shade = |col: [f32; 4], u: f32, v: f32| -> [f32; 4] {
        match tex {
            Some(img) => {
                let c = img.sample(u * s.uv_scale + s.uv_offset, v);
                [col[0] * c[0], col[1] * c[1], col[2] * c[2], col[3] * c[3]]
            }
            None => col,
        }
    };
    let coverage = |d: f32, r: f32| -> f32 {
        let aa = (r + 0.5 - d).clamp(0.0, 1.0);
        if soft > 0.0 && r > 0.5 {
            let inner = r * (1.0 - soft);
            let f = ((r - d) / (r - inner).max(1e-3)).clamp(0.0, 1.0);
            aa * f * f * (3.0 - 2.0 * f)
        } else {
            aa
        }
    };
    match mat.mode {
        LineMode::Dots | LineMode::Squares => {
            for p in &pts {
                let r = p.r.max(0.5);
                let (bx0, by0) = ((p.x - r - 1.0).floor() as i32, (p.y - r - 1.0).floor() as i32);
                let (bx1, by1) = ((p.x + r + 1.0).ceil() as i32, (p.y + r + 1.0).ceil() as i32);
                for y in by0..=by1 {
                    for x in bx0..=bx1 {
                        let (dx, dy) = (x as f32 + 0.5 - p.x, y as f32 + 0.5 - p.y);
                        let d = if mat.mode == LineMode::Dots { (dx * dx + dy * dy).sqrt() } else { dx.abs().max(dy.abs()) };
                        let c = coverage(d, r);
                        if c > 0.0 {
                            sc.put(x, y, c, shade(p.col, p.u, 0.5 + 0.5 * dx / r), p.dist);
                        }
                    }
                }
            }
        }
        LineMode::Line => {
            let n = pts.len();
            let mut segs: Vec<(usize, usize)> = (0..n.saturating_sub(1)).map(|i| (i, i + 1)).collect();
            if s.cyclic && n > 2 {
                segs.push((n - 1, 0));
            }
            if n == 1 {
                segs.push((0, 0));
            }
            let last_seg = if s.cyclic { usize::MAX } else { n.saturating_sub(2) };
            for (k, &(ia, ib)) in segs.iter().enumerate() {
                let (a, b) = (pts[ia], pts[ib]);
                let r = a.r.max(b.r);
                let bx0 = (a.x.min(b.x) - r - 1.0).floor() as i32;
                let by0 = (a.y.min(b.y) - r - 1.0).floor() as i32;
                let bx1 = (a.x.max(b.x) + r + 1.0).ceil() as i32;
                let by1 = (a.y.max(b.y) + r + 1.0).ceil() as i32;
                let (pa, pb) = (Vec2::new(a.x, a.y), Vec2::new(b.x, b.y));
                let (abx, aby) = (b.x - a.x, b.y - a.y);
                let l2 = abx * abx + aby * aby;
                let flat_start = !s.cyclic && k == 0 && s.start_cap == CapType::Flat;
                let flat_end = !s.cyclic && k == last_seg && s.end_cap == CapType::Flat;
                for y in by0.max(y0)..=by1.min(y1) {
                    for x in bx0.max(x0)..=bx1.min(x1) {
                        let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let (d, t) = seg_dist2(q, pa, pb);
                        if l2 > 1e-9 && (flat_start || flat_end) {
                            let raw = ((q.x - a.x) * abx + (q.y - a.y) * aby) / l2;
                            if (flat_start && raw < 0.0) || (flat_end && raw > 1.0) {
                                continue;
                            }
                        }
                        let rr = a.r + (b.r - a.r) * t;
                        let c = coverage(d, rr.max(0.35));
                        if c <= 0.0 {
                            continue;
                        }
                        let col = lerp4(a.col, b.col, t);
                        let u = a.u + (b.u - a.u) * t;
                        sc.put(x, y, c, shade(col, u, 0.5 + 0.5 * d / rr.max(1e-3)), a.dist + (b.dist - a.dist) * t);
                    }
                }
            }
        }
    }
    // Composite the stroke.
    let cw = canvas.w;
    for ly in 0..sc.h {
        for lx in 0..sc.w {
            let i = ly * sc.w + lx;
            let c = sc.cov[i];
            if c <= 0.0 {
                continue;
            }
            let (x, y) = ((sc.x0 + lx as i32) as usize, (sc.y0 + ly as i32) as usize);
            if !depth_ok(t, cw, x, y, sc.depth[i]) {
                continue;
            }
            if mat.use_stroke_holdout {
                canvas.cut(x, y, c);
                cut.over(x, y, [0.0, 0.0, 0.0, c]);
            } else {
                let col = sc.col[i];
                canvas.over(x, y, [col[0] * c, col[1] * c, col[2] * c, col[3] * c]);
            }
        }
    }
}

/// Fill a stroke's polygon (ear clipping) with 4x supersampled coverage.
fn draw_fill(s: &GpStroke, mat: &GpMaterial, look: &Look<'_>, t: &LayerTarget<'_>, pts: &[SP], canvas: &mut Canvas, cut: &mut Canvas) {
    let poly: Vec<Vec2> = pts.iter().map(|p| Vec2::new(p.x, p.y)).collect();
    let tris = geom::triangulate(&poly);
    if tris.is_empty() {
        return;
    }
    let (w, h) = (canvas.w as i32, canvas.h as i32);
    let bx0 = poly.iter().map(|p| p.x).fold(f32::INFINITY, f32::min).floor().max(0.0) as i32;
    let by0 = poly.iter().map(|p| p.y).fold(f32::INFINITY, f32::min).floor().max(0.0) as i32;
    let bx1 = (poly.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(w - 1);
    let by1 = (poly.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(h - 1);
    if bx0 > bx1 || by0 > by1 {
        return;
    }
    let (bw, bh) = ((bx1 - bx0 + 1) as usize, (by1 - by0 + 1) as usize);
    // 2x2 samples per pixel.
    let mut mask = vec![false; bw * bh * 4];
    for tri in &tris {
        let [a, b, c] = tri.map(|i| poly[i as usize]);
        let area = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
        if area.abs() < 1e-9 {
            continue;
        }
        let tx0 = (a.x.min(b.x).min(c.x).floor() as i32).max(bx0);
        let ty0 = (a.y.min(b.y).min(c.y).floor() as i32).max(by0);
        let tx1 = (a.x.max(b.x).max(c.x).ceil() as i32).min(bx1);
        let ty1 = (a.y.max(b.y).max(c.y).ceil() as i32).min(by1);
        for y in ty0..=ty1 {
            for x in tx0..=tx1 {
                for (k, (ox, oy)) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)].into_iter().enumerate() {
                    let (px, py) = (x as f32 + ox, y as f32 + oy);
                    let w0 = ((b.x - px) * (c.y - py) - (c.x - px) * (b.y - py)) / area;
                    let w1 = ((c.x - px) * (a.y - py) - (a.x - px) * (c.y - py)) / area;
                    if w0 >= 0.0 && w1 >= 0.0 && w0 + w1 <= 1.0 {
                        mask[((y - by0) as usize * bw + (x - bx0) as usize) * 4 + k] = true;
                    }
                }
            }
        }
    }
    let depth = pts.iter().map(|p| p.dist).sum::<f32>() / pts.len() as f32;
    let fc = s.fill_color;
    let fill_rgb = |c: [f32; 4]| mix3([c[0], c[1], c[2]], [fc[0], fc[1], fc[2]], fc[3].clamp(0.0, 1.0));
    let (gx0, gy0, gw, gh) = (bx0 as f32, by0 as f32, bw as f32, bh as f32);
    let centre = (gx0 + gw * 0.5, gy0 + gh * 0.5);
    let tex = if mat.fill_style == FillStyle::Texture { mat.fill_image.and_then(|i| look.images.get(i)) } else { None };
    let cw = canvas.w;
    for y in 0..bh {
        for x in 0..bw {
            let n = mask[(y * bw + x) * 4..(y * bw + x) * 4 + 4].iter().filter(|&&b| b).count();
            if n == 0 {
                continue;
            }
            let cov = n as f32 / 4.0;
            let (px, py) = (bx0 as usize + x, by0 as usize + y);
            if !depth_ok(t, cw, px, py, depth) {
                continue;
            }
            if mat.use_fill_holdout {
                canvas.cut(px, py, cov);
                cut.over(px, py, [0.0, 0.0, 0.0, cov]);
                continue;
            }
            let (u, v) = ((x as f32 + 0.5) / gw, 1.0 - (y as f32 + 0.5) / gh);
            let mut c = match mat.fill_style {
                FillStyle::Solid => mat.fill_color,
                FillStyle::Gradient => {
                    let k = match mat.gradient_type {
                        GradientType::Linear => {
                            let (sa, ca) = mat.texture_angle.sin_cos();
                            ((u - 0.5) * ca + (v - 0.5) * sa + 0.5).clamp(0.0, 1.0)
                        }
                        GradientType::Radial => {
                            let (dx, dy) = ((px as f32 - centre.0) / (gw * 0.5), (py as f32 - centre.1) / (gh * 0.5));
                            ((dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0)
                        }
                    };
                    let k = (k / mat.mix_factor.clamp(0.01, 1.0) * 0.5).clamp(0.0, 1.0);
                    lerp4(mat.fill_color, mat.mix_color, k)
                }
                FillStyle::Texture => match tex {
                    Some(img) => {
                        let tc = img.sample(u * mat.texture_scale[0] + mat.texture_offset[0], v * mat.texture_scale[1] + mat.texture_offset[1]);
                        tc
                    }
                    None => mat.fill_color,
                },
            };
            let rgb = mix3(fill_rgb(c), look.tint.0, look.tint.1);
            c[3] *= s.fill_opacity.clamp(0.0, 1.0);
            let (rgb, a) = match look.ghost {
                Some(g) => ([g[0], g[1], g[2]], c[3] * g[3]),
                None => (rgb, c[3]),
            };
            let a = a * cov;
            canvas.over(px, py, [rgb[0] * a, rgb[1] * a, rgb[2] * a, a]);
        }
    }
}

fn blend_layer(dst: &mut Canvas, src: &Canvas, cut: &Canvas, mode: LayerBlend, opacity: f32) {
    let (x0, y0, x1, y1) = (src.bounds.0.min(cut.bounds.0), src.bounds.1.min(cut.bounds.1), src.bounds.2.max(cut.bounds.2), src.bounds.3.max(cut.bounds.3));
    for y in y0..y1.min(dst.h) {
        for x in x0..x1.min(dst.w) {
            let i = y * dst.w + x;
            let h = cut.px[i][3];
            if h > 0.0 {
                dst.cut(x, y, h);
            }
            let s = src.px[i].map(|v| v * opacity);
            if s[3] <= 0.0 {
                continue;
            }
            let d = dst.px[i];
            let out = match mode {
                LayerBlend::Regular => {
                    let k = 1.0 - s[3];
                    [s[0] + d[0] * k, s[1] + d[1] * k, s[2] + d[2] * k, s[3] + d[3] * k]
                }
                _ => {
                    // Straight colours, blended where both exist, then over.
                    let sa = s[3];
                    let sc = [s[0] / sa, s[1] / sa, s[2] / sa];
                    let da = d[3].max(1e-6);
                    let dc = [d[0] / da, d[1] / da, d[2] / da];
                    let f = |a: f32, b: f32| match mode {
                        LayerBlend::Add => a + b,
                        LayerBlend::Subtract => (a - b).max(0.0),
                        LayerBlend::Multiply => a * b,
                        LayerBlend::Divide => if b > 1e-4 { a / b } else { a },
                        LayerBlend::HardLight => {
                            if b <= 0.5 { 2.0 * a * b } else { 1.0 - 2.0 * (1.0 - a) * (1.0 - b) }
                        }
                        LayerBlend::Regular => b,
                    };
                    let mixed = [f(dc[0], sc[0]), f(dc[1], sc[1]), f(dc[2], sc[2])];
                    // Where the destination is empty the layer shows as it is.
                    let both = d[3].min(1.0);
                    let col = mix3(sc, mixed, both);
                    let k = 1.0 - sa;
                    [col[0] * sa + d[0] * k, col[1] * sa + d[1] * k, col[2] * sa + d[2] * k, sa + d[3] * k]
                }
            };
            dst.px[i] = out;
            dst.grow(x, y);
        }
    }
}

/// Draw a Grease Pencil object's evaluated layers (plus optional onion
/// skins) into `out` (cleared first), then its visual effects.
#[allow(clippy::too_many_arguments)]
pub fn draw_object(scene: &Scene, oi: usize, gp: &GreasePencil, layers: &[EvalLayer], ghosts: &[(usize, Vec<GpStroke>, [f32; 4])], effects: &[GpEffect], view: &GpView, depth: Option<&[f32]>, out: &mut Canvas, render: bool) {
    let (w, h) = (view.width as usize, view.height as usize);
    if out.w != w || out.h != h {
        *out = Canvas::new(w, h);
    }
    out.clear();
    let mut layer = Canvas::new(w, h);
    let mut cut = Canvas::new(w, h);
    let mut sc = Scratch::default();
    let images = &scene.images;
    for (li, l) in gp.layers.iter().enumerate() {
        if !gp.layer_visible(li) {
            continue;
        }
        let target = LayerTarget { view, depth, world: super::layer_world_matrix(scene, oi, gp, li) };
        layer.clear();
        cut.clear();
        for (gl, strokes, color) in ghosts {
            if *gl != li {
                continue;
            }
            let look = Look { materials: &gp.materials, images, tint: (l.tint_color, 0.0), ghost: Some(*color) };
            for s in strokes {
                draw_stroke(s, &look, &target, l.radius_offset, &mut layer, &mut cut, &mut sc);
            }
        }
        if let Some(el) = layers.iter().find(|e| e.layer == li) {
            let look = Look { materials: &gp.materials, images, tint: (l.tint_color, l.tint_factor), ghost: None };
            for s in &el.strokes {
                draw_stroke(s, &look, &target, l.radius_offset, &mut layer, &mut cut, &mut sc);
            }
        }
        blend_layer(out, &layer, &cut, l.blend, l.opacity.clamp(0.0, 1.0));
    }
    let fx: Vec<&GpEffect> = effects.iter().filter(|e| if render { e.show_render } else { e.show_viewport }).collect();
    if !fx.is_empty() && !out.is_empty() {
        let origin = view.project(scene.world_matrix(oi).transform_point(crate::math::Vec3::ZERO)).map(|p| (p.0, p.1));
        apply_effects(out, &fx, origin, scene.frame_current);
    }
}

/// Onion skin strokes of each layer around `frame`: (layer, strokes, colour with alpha).
pub fn onion_ghosts(gp: &GreasePencil, frame: i32) -> Vec<(usize, Vec<GpStroke>, [f32; 4])> {
    let o = &gp.onion;
    let mut out = Vec::new();
    if !o.enabled {
        return out;
    }
    for (li, l) in gp.layers.iter().enumerate() {
        if !l.use_onion_skinning || l.frames.is_empty() {
            continue;
        }
        let cur = l.frame_index_at(frame);
        let mut add = |fi: usize, k: u32, before: bool, n: u32| {
            let fade = if o.use_fade { 1.0 - (k as f32 - 1.0) / n.max(1) as f32 } else { 1.0 };
            let c = if before { o.color_before } else { o.color_after };
            let c = if o.use_colors { c } else { [0.5; 3] };
            out.push((li, l.frames[fi].drawing.strokes.clone(), [c[0], c[1], c[2], o.opacity * fade.clamp(0.1, 1.0)]));
        };
        match o.mode {
            OnionMode::Keyframes | OnionMode::Selected => {
                let sel = o.mode == OnionMode::Selected;
                let ci = cur.map_or(-1, |c| c as i64);
                let mut k = 0;
                let mut i = ci - 1;
                while i >= 0 && k < o.before {
                    if !sel || l.frames[i as usize].select {
                        k += 1;
                        add(i as usize, k, true, o.before);
                    }
                    i -= 1;
                }
                let mut k = 0;
                let mut i = (ci + 1) as usize;
                while i < l.frames.len() && k < o.after {
                    if !sel || l.frames[i].select {
                        k += 1;
                        add(i, k, false, o.after);
                    }
                    i += 1;
                }
            }
            OnionMode::Frames => {
                for k in 1..=o.before {
                    if let Some(fi) = l.frame_index_at(frame - k as i32).filter(|&fi| Some(fi) != cur) {
                        add(fi, k, true, o.before);
                    }
                }
                for k in 1..=o.after {
                    if let Some(fi) = l.frame_index_at(frame + k as i32).filter(|&fi| Some(fi) != cur) {
                        add(fi, k, false, o.after);
                    }
                }
            }
        }
    }
    out
}

/// Grease Pencil objects of a scene in drawing order (farthest first).
pub fn objects_back_to_front(scene: &Scene, view: &GpView) -> Vec<usize> {
    let mut v: Vec<(f32, usize)> = (0..scene.objects.len())
        .filter(|&i| matches!(scene.objects[i].data, ObjectData::GreasePencil(g) if g < scene.grease_pencils.len()) && scene.is_visible(i))
        .map(|i| (-view.to_view(scene.world_matrix(i).transform_point(crate::math::Vec3::ZERO)).z, i))
        .collect();
    v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    v.into_iter().map(|x| x.1).collect()
}

/// Options for drawing every Grease Pencil object of a scene.
pub struct SceneDraw<'a> {
    pub view: &'a GpView,
    pub depth: Option<&'a [f32]>,
    pub frame: i32,
    pub render: bool,
    pub onion: bool,
    /// Pre-evaluated layers per object (the viewport's cache); evaluated here when missing.
    pub evaluated: Option<&'a dyn Fn(usize) -> Option<Vec<EvalLayer>>>,
}

/// Where object canvases are composited.
pub trait Target {
    /// Source-over premultiplied linear `src` at pixel `i`.
    fn over(&mut self, i: usize, src: [f32; 4]);
}

impl Target for [[f32; 4]] {
    fn over(&mut self, i: usize, s: [f32; 4]) {
        let d = &mut self[i];
        let k = 1.0 - s[3].clamp(0.0, 1.0);
        for j in 0..4 {
            d[j] = s[j] + d[j] * k;
        }
    }
}

/// An sRGB RGBA8 image (the viewport's), blended in linear light.
pub struct Srgb8<'a> {
    pub rgba: &'a mut [u8],
    lut: Vec<f32>,
}

impl<'a> Srgb8<'a> {
    pub fn new(rgba: &'a mut [u8]) -> Srgb8<'a> {
        Srgb8 { rgba, lut: (0..256).map(|i| crate::render::srgb_decode(i as f32 / 255.0)).collect() }
    }
}

impl Target for Srgb8<'_> {
    fn over(&mut self, i: usize, s: [f32; 4]) {
        let k = 1.0 - s[3].clamp(0.0, 1.0);
        let enc = |v: f32| (crate::render::srgb_encode(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
        for j in 0..3 {
            let d = self.lut[self.rgba[i * 4 + j] as usize];
            self.rgba[i * 4 + j] = enc(s[j] + d * k);
        }
    }
}

/// Draw every Grease Pencil object and composite each over `dst`, back to
/// front. Returns how many objects drew something.
pub fn composite_scene(scene: &Scene, opts: &SceneDraw<'_>, dst: &mut (impl Target + ?Sized)) -> usize {
    let mut canvas = Canvas::default();
    let mut n = 0;
    for oi in objects_back_to_front(scene, opts.view) {
        let ObjectData::GreasePencil(gi) = scene.objects[oi].data else { continue };
        let gp = &scene.grease_pencils[gi];
        let ob = &scene.objects[oi];
        let layers = opts.evaluated.and_then(|f| f(oi)).unwrap_or_else(|| evaluate(gp, &ob.gp_modifiers, &EvalContext { scene: Some(scene), owner: oi, frame: opts.frame, render: opts.render }));
        let ghosts = if opts.onion && scene.active_object == Some(oi) { onion_ghosts(gp, opts.frame) } else { Vec::new() };
        draw_object(scene, oi, gp, &layers, &ghosts, &ob.shader_fx, opts.view, opts.depth, &mut canvas, opts.render);
        if canvas.is_empty() {
            continue;
        }
        let (x0, y0, x1, y1) = canvas.bounds;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = y * canvas.w + x;
                let s = canvas.px[i];
                if s == [0.0; 4] {
                    continue;
                }
                dst.over(i, s);
            }
        }
        n += 1;
    }
    n
}

/// Composite Grease Pencil over an F12 render (linear RGBA, premultiplied by
/// coverage) using its depth pass, from the scene camera.
pub fn composite_render(scene: &Scene, image: &mut Image, depth: Option<&Image>, frame: i32) -> usize {
    let Some(ci) = scene.active_camera else { return 0 };
    let ObjectData::Camera(cam) = &scene.objects[ci].data else { return 0 };
    if !scene.objects.iter().any(|o| matches!(o.data, ObjectData::GreasePencil(_))) {
        return 0;
    }
    let view = GpView::from_camera(&scene.world_matrix(ci), cam, image.width, image.height);
    let z: Option<Vec<f32>> = depth.filter(|d| d.width == image.width && d.height == image.height).map(|d| d.pixels.iter().map(|p| p[0]).collect());
    let opts = SceneDraw { view: &view, depth: z.as_deref(), frame, render: true, onion: false, evaluated: None };
    composite_scene(scene, &opts, image.pixels.as_mut_slice())
}

/// Convert the Solid viewport's depth keys (1/distance in perspective, view
/// z in ortho; larger is closer, -inf empty) to distances.
pub fn viewport_depth_to_distance(keys: &[f32], ortho: bool) -> Vec<f32> {
    keys.iter()
        .map(|&k| {
            if !k.is_finite() {
                f32::INFINITY
            } else if ortho {
                -k
            } else if k > 0.0 {
                1.0 / k
            } else {
                f32::INFINITY
            }
        })
        .collect()
}

/// Composite onto an sRGB RGBA8 buffer (the viewport image), in linear light.
pub fn composite_srgb8(scene: &Scene, opts: &SceneDraw<'_>, rgba: &mut [u8]) -> usize {
    if rgba.len() < (opts.view.width * opts.view.height) as usize * 4 {
        return 0;
    }
    composite_scene(scene, opts, &mut Srgb8::new(rgba))
}
