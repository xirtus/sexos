// SPDX-License-Identifier: GPL-3.0-only
//! Primitive assembly, clipping and rasterization: triangles (with near/far clipping,
//! culling, perspective-correct varyings), lines and points; depth test, blending and
//! color masks on an RGBA8 + f32 depth framebuffer. Rows are stored bottom-up (GL order).

use alloc::vec;
use alloc::vec::Vec;

/// A shaded vertex: clip position, point size and packed varyings.
#[derive(Clone)]
pub struct Vert {
    pub pos: [f32; 4],
    pub point_size: f32,
    pub vary: Vec<f32>,
}

/// A target to draw into.
pub struct Target<'a> {
    pub width: i32,
    pub height: i32,
    pub color: &'a mut [u32],
    pub depth: &'a mut [f32],
}

/// Fixed-function state for one draw.
#[derive(Clone, Copy, Debug)]
pub struct Fixed {
    pub viewport: [i32; 4],
    pub scissor: Option<[i32; 4]>,
    pub depth_test: bool,
    pub depth_func: u32,
    pub depth_mask: bool,
    pub depth_range: [f32; 2],
    pub blend: bool,
    pub blend_src: [u32; 2],
    pub blend_dst: [u32; 2],
    pub blend_eq: [u32; 2],
    pub blend_color: [f32; 4],
    pub color_mask: [bool; 4],
    pub cull: bool,
    pub cull_face: u32,
    pub front_ccw: bool,
    pub polygon_offset: Option<[f32; 2]>,
    /// Clip-space depth runs 0..w (WebGPU) instead of -w..w (GL).
    pub zero_one: bool,
}

pub mod gl {
    pub const NEVER: u32 = 0x0200;
    pub const LESS: u32 = 0x0201;
    pub const EQUAL: u32 = 0x0202;
    pub const LEQUAL: u32 = 0x0203;
    pub const GREATER: u32 = 0x0204;
    pub const NOTEQUAL: u32 = 0x0205;
    pub const GEQUAL: u32 = 0x0206;
    pub const ALWAYS: u32 = 0x0207;
    pub const ZERO: u32 = 0;
    pub const ONE: u32 = 1;
    pub const SRC_COLOR: u32 = 0x0300;
    pub const ONE_MINUS_SRC_COLOR: u32 = 0x0301;
    pub const SRC_ALPHA: u32 = 0x0302;
    pub const ONE_MINUS_SRC_ALPHA: u32 = 0x0303;
    pub const DST_ALPHA: u32 = 0x0304;
    pub const ONE_MINUS_DST_ALPHA: u32 = 0x0305;
    pub const DST_COLOR: u32 = 0x0306;
    pub const ONE_MINUS_DST_COLOR: u32 = 0x0307;
    pub const SRC_ALPHA_SATURATE: u32 = 0x0308;
    pub const CONSTANT_COLOR: u32 = 0x8001;
    pub const ONE_MINUS_CONSTANT_COLOR: u32 = 0x8002;
    pub const CONSTANT_ALPHA: u32 = 0x8003;
    pub const ONE_MINUS_CONSTANT_ALPHA: u32 = 0x8004;
    pub const FUNC_ADD: u32 = 0x8006;
    pub const MIN: u32 = 0x8007;
    pub const MAX: u32 = 0x8008;
    pub const FUNC_SUBTRACT: u32 = 0x800A;
    pub const FUNC_REVERSE_SUBTRACT: u32 = 0x800B;
    pub const FRONT: u32 = 0x0404;
    pub const BACK: u32 = 0x0405;
    pub const FRONT_AND_BACK: u32 = 0x0408;
    pub const POINTS: u32 = 0;
    pub const LINES: u32 = 1;
    pub const LINE_LOOP: u32 = 2;
    pub const LINE_STRIP: u32 = 3;
    pub const TRIANGLES: u32 = 4;
    pub const TRIANGLE_STRIP: u32 = 5;
    pub const TRIANGLE_FAN: u32 = 6;
}

/// What the rasterizer asks of a fragment: shade it, or `None` when discarded. Gets
/// window x, y (pixel centres), z, 1/w, front-facing, point coordinate and varyings.
pub trait Shade {
    fn shade(&mut self, frag: &Frag, vary: &[f32]) -> Option<([f32; 4], Option<f32>)>;
    /// True if the shader writes depth: the depth test then waits for its value.
    fn writes_depth(&self) -> bool { false }
    /// How many fragments `shade_many` takes at once (1: it isn't used).
    fn batch(&self) -> usize { 1 }
    /// Shades `frags` (at most `batch()`), fragment `i` with varyings `vary[i * stride..]`.
    fn shade_many(&mut self, frags: &[Frag], vary: &[f32], stride: usize, out: &mut [Option<([f32; 4], Option<f32>)>]) {
        for (i, f) in frags.iter().enumerate() { out[i] = self.shade(f, &vary[i * stride..(i + 1) * stride]); }
    }
}

/// Fragments of one primitive waiting to be shaded together. They cover different pixels,
/// so shading them as a group and then writing them in order is the same as one by one.
#[derive(Default)]
pub struct Batch {
    frags: Vec<Frag>,
    at: Vec<(usize, f32)>,
    vary: Vec<f32>,
    out: Vec<Option<([f32; 4], Option<f32>)>>,
}

impl Batch {
    fn push(&mut self, t: &mut Target, fx: &Fixed, i: usize, z: f32, sh: &mut dyn Shade, frag: Frag, vary: &[f32]) {
        if fx.depth_test && !sh.writes_depth() && !depth_pass(fx.depth_func, z, t.depth[i]) { return; }
        self.frags.push(frag);
        self.at.push((i, z));
        self.vary.extend_from_slice(vary);
        if self.frags.len() >= sh.batch() { self.flush(t, fx, sh, vary.len()); }
    }

    fn flush(&mut self, t: &mut Target, fx: &Fixed, sh: &mut dyn Shade, stride: usize) {
        if self.frags.is_empty() { return; }
        self.out.clear();
        self.out.resize(self.frags.len(), None);
        sh.shade_many(&self.frags, &self.vary, stride, &mut self.out);
        for k in 0..self.frags.len() {
            let (i, z) = self.at[k];
            if let Some(r) = self.out[k] { finish(t, fx, i, z, r); }
        }
        self.frags.clear();
        self.at.clear();
        self.vary.clear();
    }
}

pub struct Frag {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub inv_w: f32,
    pub front: bool,
    pub point: [f32; 2],
}

fn depth_pass(func: u32, z: f32, d: f32) -> bool {
    match func {
        gl::NEVER => false,
        gl::LESS => z < d,
        gl::EQUAL => z == d,
        gl::LEQUAL => z <= d,
        gl::GREATER => z > d,
        gl::NOTEQUAL => z != d,
        gl::GEQUAL => z >= d,
        _ => true,
    }
}

fn unpack(p: u32) -> [f32; 4] {
    [(p & 0xFF) as f32 / 255.0, ((p >> 8) & 0xFF) as f32 / 255.0, ((p >> 16) & 0xFF) as f32 / 255.0, (p >> 24) as f32 / 255.0]
}

pub fn pack(c: [f32; 4]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
    b(c[0]) | b(c[1]) << 8 | b(c[2]) << 16 | b(c[3]) << 24
}

fn factor(f: u32, s: [f32; 4], d: [f32; 4], k: [f32; 4], ch: usize) -> f32 {
    match f {
        gl::ZERO => 0.0,
        gl::ONE => 1.0,
        gl::SRC_COLOR => s[ch],
        gl::ONE_MINUS_SRC_COLOR => 1.0 - s[ch],
        gl::SRC_ALPHA => s[3],
        gl::ONE_MINUS_SRC_ALPHA => 1.0 - s[3],
        gl::DST_ALPHA => d[3],
        gl::ONE_MINUS_DST_ALPHA => 1.0 - d[3],
        gl::DST_COLOR => d[ch],
        gl::ONE_MINUS_DST_COLOR => 1.0 - d[ch],
        gl::SRC_ALPHA_SATURATE => if ch == 3 { 1.0 } else { s[3].min(1.0 - d[3]) },
        gl::CONSTANT_COLOR => k[ch],
        gl::ONE_MINUS_CONSTANT_COLOR => 1.0 - k[ch],
        gl::CONSTANT_ALPHA => k[3],
        gl::ONE_MINUS_CONSTANT_ALPHA => 1.0 - k[3],
        _ => 1.0,
    }
}

fn blend(fx: &Fixed, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    let s = [s[0].clamp(0.0, 1.0), s[1].clamp(0.0, 1.0), s[2].clamp(0.0, 1.0), s[3].clamp(0.0, 1.0)];
    let mut o = [0f32; 4];
    for ch in 0..4 {
        let a = (ch == 3) as usize;
        let (sf, df) = (factor(fx.blend_src[a], s, d, fx.blend_color, ch), factor(fx.blend_dst[a], s, d, fx.blend_color, ch));
        o[ch] = match fx.blend_eq[a] {
            gl::FUNC_SUBTRACT => s[ch] * sf - d[ch] * df,
            gl::FUNC_REVERSE_SUBTRACT => d[ch] * df - s[ch] * sf,
            gl::MIN => s[ch].min(d[ch]),
            gl::MAX => s[ch].max(d[ch]),
            _ => s[ch] * sf + d[ch] * df,
        };
    }
    o
}

/// The drawable rectangle: viewport-independent framebuffer bounds and scissor.
fn bounds(t: &Target, fx: &Fixed) -> (i32, i32, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = (0, 0, t.width, t.height);
    if let Some(s) = fx.scissor {
        x0 = x0.max(s[0]);
        y0 = y0.max(s[1]);
        x1 = x1.min(s[0] + s[2]);
        y1 = y1.min(s[1] + s[3]);
    }
    (x0, y0, x1, y1)
}

/// Writes one shaded fragment, after the depth test.
fn write(t: &mut Target, fx: &Fixed, x: i32, y: i32, z: f32, sh: &mut dyn Shade, frag: &Frag, vary: &[f32]) {
    let i = (y * t.width + x) as usize;
    if fx.depth_test && !sh.writes_depth() && !depth_pass(fx.depth_func, z, t.depth[i]) { return; }
    let Some(r) = sh.shade(frag, vary) else { return };
    finish(t, fx, i, z, r);
}

/// Writes a shaded fragment at pixel `i` (depth `z` before the shader).
fn finish(t: &mut Target, fx: &Fixed, i: usize, z: f32, (c, fz): ([f32; 4], Option<f32>)) {
    let z = fz.unwrap_or(z);
    if fx.depth_test && fz.is_some() && !depth_pass(fx.depth_func, z, t.depth[i]) { return; }
    if fx.depth_test && fx.depth_mask { t.depth[i] = z; }
    let dst = t.color[i];
    let c = if fx.blend { blend(fx, c, unpack(dst)) } else { c };
    let mut out = pack(c);
    if fx.color_mask != [true; 4] {
        let mut m = 0u32;
        for ch in 0..4 { if fx.color_mask[ch] { m |= 0xFF << (ch * 8); } }
        out = (out & m) | (dst & !m);
    }
    t.color[i] = out;
}

/// Window coordinates of a clip-space vertex: x, y, z (depth range), 1/w.
fn window(fx: &Fixed, p: [f32; 4]) -> [f32; 4] {
    let iw = 1.0 / p[3];
    let (nx, ny, nz) = (p[0] * iw, p[1] * iw, p[2] * iw);
    let nz = if fx.zero_one { nz * 2.0 - 1.0 } else { nz };
    let [vx, vy, vw, vh] = fx.viewport;
    [
        vx as f32 + (nx + 1.0) * 0.5 * vw as f32,
        vy as f32 + (ny + 1.0) * 0.5 * vh as f32,
        fx.depth_range[0] + (nz + 1.0) * 0.5 * (fx.depth_range[1] - fx.depth_range[0]),
        iw,
    ]
}

fn lerp_vert(a: &Vert, b: &Vert, t: f32) -> Vert {
    let mut pos = [0f32; 4];
    for i in 0..4 { pos[i] = a.pos[i] + (b.pos[i] - a.pos[i]) * t; }
    Vert { pos, point_size: a.point_size, vary: a.vary.iter().zip(&b.vary).map(|(x, y)| x + (y - x) * t).collect() }
}

/// Clips a triangle against the near (z >= -w) and far (z <= w) planes.
fn clip(tri: [&Vert; 3], zero_one: bool) -> Vec<Vert> {
    let mut poly: Vec<Vert> = tri.iter().map(|v| (*v).clone()).collect();
    for plane in 0..2 {
        let dist = |v: &Vert| if plane == 0 { if zero_one { v.pos[2] } else { v.pos[2] + v.pos[3] } } else { v.pos[3] - v.pos[2] };
        if poly.iter().all(|v| dist(v) >= 0.0) { continue; }
        let mut out = Vec::with_capacity(poly.len() + 2);
        for i in 0..poly.len() {
            let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
            let (da, db) = (dist(a), dist(b));
            if da >= 0.0 { out.push(a.clone()); }
            if (da >= 0.0) != (db >= 0.0) { out.push(lerp_vert(a, b, da / (da - db))); }
        }
        poly = out;
        if poly.len() < 3 { return Vec::new(); }
    }
    poly
}

pub fn triangle(t: &mut Target, fx: &Fixed, a: &Vert, b: &Vert, c: &Vert, sh: &mut dyn Shade, scratch: &mut Vec<f32>) {
    // Trivial reject: all outside one side plane.
    for axis in 0..2 {
        if [a, b, c].iter().all(|v| v.pos[axis] > v.pos[3]) || [a, b, c].iter().all(|v| v.pos[axis] < -v.pos[3]) { return; }
    }
    let near = |v: &Vert| if fx.zero_one { 0.0 } else { -v.pos[3] };
    let needs_clip = [a, b, c].iter().any(|v| v.pos[2] < near(v) || v.pos[2] > v.pos[3] || v.pos[3] <= 0.0);
    if needs_clip {
        let poly = clip([a, b, c], fx.zero_one);
        // The winding is decided on the unclipped triangle in clip space via the first fan.
        for i in 1..poly.len().saturating_sub(1) { raster_tri(t, fx, &poly[0], &poly[i], &poly[i + 1], sh, scratch); }
    } else {
        raster_tri(t, fx, a, b, c, sh, scratch);
    }
}

fn raster_tri(t: &mut Target, fx: &Fixed, a: &Vert, b: &Vert, c: &Vert, sh: &mut dyn Shade, scratch: &mut Vec<f32>) {
    let (wa, wb, wc) = (window(fx, a.pos), window(fx, b.pos), window(fx, c.pos));
    let area = (wb[0] - wa[0]) * (wc[1] - wa[1]) - (wc[0] - wa[0]) * (wb[1] - wa[1]);
    if !(area.abs() > 1e-12) { return; }
    let ccw = area > 0.0;
    let front = ccw == fx.front_ccw;
    if fx.cull {
        match fx.cull_face {
            gl::FRONT if front => return,
            gl::BACK if !front => return,
            gl::FRONT_AND_BACK => return,
            _ => {}
        }
    }
    let (x0, y0, x1, y1) = bounds(t, fx);
    let minx = (libm::floorf(wa[0].min(wb[0]).min(wc[0])) as i32).max(x0);
    let maxx = (libm::ceilf(wa[0].max(wb[0]).max(wc[0])) as i32).min(x1 - 1);
    let miny = (libm::floorf(wa[1].min(wb[1]).min(wc[1])) as i32).max(y0);
    let maxy = (libm::ceilf(wa[1].max(wb[1]).max(wc[1])) as i32).min(y1 - 1);
    if minx > maxx || miny > maxy { return; }
    // Orient so the edge functions are positive inside.
    let (p0, p1, p2, v0, v1, v2) = if ccw { (wa, wb, wc, a, b, c) } else { (wa, wc, wb, a, c, b) };
    let edge = |p: [f32; 4], q: [f32; 4], x: f32, y: f32| (q[0] - p[0]) * (y - p[1]) - (q[1] - p[1]) * (x - p[0]);
    // Top-left rule: an edge owns its pixels if it is a left edge or a top edge.
    let owns = |p: [f32; 4], q: [f32; 4]| (q[1] < p[1]) || (q[1] == p[1] && q[0] < p[0]);
    let (o0, o1, o2) = (owns(p1, p2), owns(p2, p0), owns(p0, p1));
    let total = edge(p0, p1, p2[0], p2[1]);
    let inv_total = 1.0 / total;
    let n = v0.vary.len();
    scratch.clear();
    scratch.resize(n, 0.0);
    let offset = match fx.polygon_offset {
        Some([factor, units]) => {
            // A coarse slope term from the depth gradient.
            let dzdx = ((p1[2] - p0[2]) * (p2[1] - p0[1]) - (p2[2] - p0[2]) * (p1[1] - p0[1])) * inv_total;
            let dzdy = ((p2[2] - p0[2]) * (p1[0] - p0[0]) - (p1[2] - p0[2]) * (p2[0] - p0[0])) * inv_total;
            factor * dzdx.abs().max(dzdy.abs()) + units * (1.0 / 16_777_216.0)
        }
        None => 0.0,
    };
    let mut batch = Batch::default();
    let wide = sh.batch() > 1;
    for y in miny..=maxy {
        let py = y as f32 + 0.5;
        for x in minx..=maxx {
            let px = x as f32 + 0.5;
            let e0 = edge(p1, p2, px, py);
            let e1 = edge(p2, p0, px, py);
            let e2 = edge(p0, p1, px, py);
            if e0 < 0.0 || e1 < 0.0 || e2 < 0.0 { continue; }
            if (e0 == 0.0 && !o0) || (e1 == 0.0 && !o1) || (e2 == 0.0 && !o2) { continue; }
            let (l0, l1, l2) = (e0 * inv_total, e1 * inv_total, e2 * inv_total);
            let z = l0 * p0[2] + l1 * p1[2] + l2 * p2[2] + offset;
            let iw = l0 * p0[3] + l1 * p1[3] + l2 * p2[3];
            let (q0, q1, q2) = (l0 * p0[3] / iw, l1 * p1[3] / iw, l2 * p2[3] / iw);
            for k in 0..n { scratch[k] = q0 * v0.vary[k] + q1 * v1.vary[k] + q2 * v2.vary[k]; }
            let frag = Frag { x: px, y: py, z, inv_w: iw, front, point: [0.0, 0.0] };
            if wide {
                batch.push(t, fx, (y * t.width + x) as usize, z.clamp(0.0, 1.0), sh, frag, scratch);
            } else {
                write(t, fx, x, y, z.clamp(0.0, 1.0), sh, &frag, scratch);
            }
        }
    }
    batch.flush(t, fx, sh, n);
}

pub fn line(t: &mut Target, fx: &Fixed, a: &Vert, b: &Vert, sh: &mut dyn Shade, scratch: &mut Vec<f32>) {
    if a.pos[3] <= 0.0 || b.pos[3] <= 0.0 { return; }
    let (wa, wb) = (window(fx, a.pos), window(fx, b.pos));
    let (dx, dy) = (wb[0] - wa[0], wb[1] - wa[1]);
    let steps = libm::ceilf(dx.abs().max(dy.abs())).max(1.0) as i32;
    let (x0, y0, x1, y1) = bounds(t, fx);
    let n = a.vary.len();
    scratch.clear();
    scratch.resize(n, 0.0);
    for s in 0..steps {
        let k = (s as f32 + 0.5) / steps as f32;
        let (x, y) = (libm::floorf(wa[0] + dx * k) as i32, libm::floorf(wa[1] + dy * k) as i32);
        if x < x0 || y < y0 || x >= x1 || y >= y1 { continue; }
        let iw = wa[3] + (wb[3] - wa[3]) * k;
        let (qa, qb) = ((1.0 - k) * wa[3] / iw, k * wb[3] / iw);
        for j in 0..n { scratch[j] = qa * a.vary[j] + qb * b.vary[j]; }
        let z = wa[2] + (wb[2] - wa[2]) * k;
        let frag = Frag { x: x as f32 + 0.5, y: y as f32 + 0.5, z, inv_w: iw, front: true, point: [0.0, 0.0] };
        write(t, fx, x, y, z.clamp(0.0, 1.0), sh, &frag, scratch);
    }
}

pub fn point(t: &mut Target, fx: &Fixed, a: &Vert, sh: &mut dyn Shade) {
    if a.pos[3] <= 0.0 { return; }
    let w = window(fx, a.pos);
    let size = a.point_size.clamp(1.0, 256.0);
    let (x0, y0, x1, y1) = bounds(t, fx);
    let left = libm::floorf(w[0] - size / 2.0 + 0.5) as i32;
    let bottom = libm::floorf(w[1] - size / 2.0 + 0.5) as i32;
    let s = size as i32;
    for y in bottom.max(y0)..(bottom + s).min(y1) {
        for x in left.max(x0)..(left + s).min(x1) {
            let pc = [((x as f32 + 0.5) - (w[0] - size / 2.0)) / size, 1.0 - ((y as f32 + 0.5) - (w[1] - size / 2.0)) / size];
            let frag = Frag { x: x as f32 + 0.5, y: y as f32 + 0.5, z: w[2], inv_w: w[3], front: true, point: pc };
            write(t, fx, x, y, w[2].clamp(0.0, 1.0), sh, &frag, &a.vary);
        }
    }
}

/// Assembles `mode` primitives from `idx` (indices into `verts`).
pub fn assemble(t: &mut Target, fx: &Fixed, mode: u32, idx: &[u32], verts: &[Vert], sh: &mut dyn Shade) {
    let v = |i: usize| &verts[idx[i] as usize];
    let mut scratch = vec![];
    let n = idx.len();
    match mode {
        gl::TRIANGLES => for i in (0..n / 3 * 3).step_by(3) { triangle(t, fx, v(i), v(i + 1), v(i + 2), sh, &mut scratch); },
        gl::TRIANGLE_STRIP => for i in 0..n.saturating_sub(2) {
            if i % 2 == 0 { triangle(t, fx, v(i), v(i + 1), v(i + 2), sh, &mut scratch) } else { triangle(t, fx, v(i + 1), v(i), v(i + 2), sh, &mut scratch) }
        },
        gl::TRIANGLE_FAN => for i in 1..n.saturating_sub(1) { triangle(t, fx, v(0), v(i), v(i + 1), sh, &mut scratch); },
        gl::LINES => for i in (0..n / 2 * 2).step_by(2) { line(t, fx, v(i), v(i + 1), sh, &mut scratch); },
        gl::LINE_STRIP => for i in 0..n.saturating_sub(1) { line(t, fx, v(i), v(i + 1), sh, &mut scratch); },
        gl::LINE_LOOP => {
            for i in 0..n.saturating_sub(1) { line(t, fx, v(i), v(i + 1), sh, &mut scratch); }
            if n > 2 { line(t, fx, v(n - 1), v(0), sh, &mut scratch); }
        }
        gl::POINTS => for i in 0..n { point(t, fx, v(i), sh); },
        _ => {}
    }
}
