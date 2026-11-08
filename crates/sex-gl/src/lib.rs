// SPDX-License-Identifier: GPL-3.0-only
//! Software WebGL: GLSL ES shaders translated for naga and run by an interpreter, drawn
//! by a rasterizer with depth testing, blending and textures.
//!
//! [`Context`] mirrors the WebGL 1 API with integer object names chosen by the caller.
#![no_std]
extern crate alloc;

pub mod exec;
pub mod glsl;
pub mod gpu;
pub mod program;
pub mod raster;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use exec::{Io, Textures};
use glsl::{SamplerKind, Stage};
use program::{Program, Shader};
use raster::{gl as rg, Fixed, Frag, Shade, Target, Vert};

/// WebGL enums the context reads.
pub mod enums {
    pub const DEPTH_BUFFER_BIT: u32 = 0x0100;
    pub const STENCIL_BUFFER_BIT: u32 = 0x0400;
    pub const COLOR_BUFFER_BIT: u32 = 0x4000;
    pub const ARRAY_BUFFER: u32 = 0x8892;
    pub const ELEMENT_ARRAY_BUFFER: u32 = 0x8893;
    pub const BYTE: u32 = 0x1400;
    pub const UNSIGNED_BYTE: u32 = 0x1401;
    pub const SHORT: u32 = 0x1402;
    pub const UNSIGNED_SHORT: u32 = 0x1403;
    pub const INT: u32 = 0x1404;
    pub const UNSIGNED_INT: u32 = 0x1405;
    pub const FLOAT: u32 = 0x1406;
    pub const HALF_FLOAT: u32 = 0x140B;
    pub const HALF_FLOAT_OES: u32 = 0x8D61;
    pub const UNSIGNED_SHORT_4_4_4_4: u32 = 0x8033;
    pub const UNSIGNED_SHORT_5_5_5_1: u32 = 0x8034;
    pub const UNSIGNED_SHORT_5_6_5: u32 = 0x8363;
    pub const CULL_FACE: u32 = 0x0B44;
    pub const BLEND: u32 = 0x0BE2;
    pub const DEPTH_TEST: u32 = 0x0B71;
    pub const SCISSOR_TEST: u32 = 0x0C11;
    pub const POLYGON_OFFSET_FILL: u32 = 0x8037;
    pub const CW: u32 = 0x0900;
    pub const CCW: u32 = 0x0901;
    pub const TEXTURE_2D: u32 = 0x0DE1;
    pub const TEXTURE_CUBE_MAP: u32 = 0x8513;
    pub const TEXTURE_CUBE_MAP_POSITIVE_X: u32 = 0x8515;
    pub const TEXTURE0: u32 = 0x84C0;
    pub const TEXTURE_MAG_FILTER: u32 = 0x2800;
    pub const TEXTURE_MIN_FILTER: u32 = 0x2801;
    pub const TEXTURE_WRAP_S: u32 = 0x2802;
    pub const TEXTURE_WRAP_T: u32 = 0x2803;
    pub const NEAREST: u32 = 0x2600;
    pub const LINEAR: u32 = 0x2601;
    pub const REPEAT: u32 = 0x2901;
    pub const CLAMP_TO_EDGE: u32 = 0x812F;
    pub const MIRRORED_REPEAT: u32 = 0x8370;
    pub const ALPHA: u32 = 0x1906;
    pub const RGB: u32 = 0x1907;
    pub const RGBA: u32 = 0x1908;
    pub const LUMINANCE: u32 = 0x1909;
    pub const LUMINANCE_ALPHA: u32 = 0x190A;
    pub const RED: u32 = 0x1903;
    pub const RG: u32 = 0x8227;
    pub const UNPACK_ALIGNMENT: u32 = 0x0CF5;
    pub const UNPACK_FLIP_Y_WEBGL: u32 = 0x9240;
    pub const UNPACK_PREMULTIPLY_ALPHA_WEBGL: u32 = 0x9241;
    pub const NO_ERROR: u32 = 0;
    pub const INVALID_ENUM: u32 = 0x0500;
    pub const INVALID_VALUE: u32 = 0x0501;
    pub const INVALID_OPERATION: u32 = 0x0502;
    pub const VERTEX_SHADER: u32 = 0x8B31;
    pub const FRAGMENT_SHADER: u32 = 0x8B30;
}
use enums::*;

pub const MAX_ATTRIBS: usize = 16;
pub const MAX_UNITS: usize = 16;
/// Largest texture or drawing-buffer side.
pub const MAX_SIZE: u32 = 4096;

#[derive(Clone, Copy, Debug)]
pub struct Attrib {
    pub enabled: bool,
    pub buffer: u32,
    pub size: u32,
    pub ty: u32,
    pub normalized: bool,
    pub stride: u32,
    pub offset: u32,
    pub divisor: u32,
    /// Integer components reach the shader as integers (vertexAttribIPointer, WebGPU
    /// uint/sint formats), not converted to floats.
    pub integer: bool,
}

impl Default for Attrib {
    fn default() -> Self { Self { enabled: false, buffer: 0, size: 4, ty: FLOAT, normalized: false, stride: 0, offset: 0, divisor: 0, integer: false } }
}

#[derive(Clone, Default)]
pub struct Vao {
    pub element: u32,
    pub attribs: [Attrib; MAX_ATTRIBS],
}

/// One texture image (RGBA8, `r | g << 8 | b << 16 | a << 24`; row 0 is t = 0).
#[derive(Clone, Default)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub px: Vec<u32>,
}

#[derive(Clone)]
pub struct Texture {
    pub target: u32,
    /// Level 0 of each face (one for 2D, six for cube maps).
    pub faces: Vec<Level>,
    pub min_filter: u32,
    pub mag_filter: u32,
    pub wrap_s: u32,
    pub wrap_t: u32,
}

impl Texture {
    fn new(target: u32) -> Self {
        Texture { target, faces: vec![Level::default(); if target == TEXTURE_CUBE_MAP { 6 } else { 1 }], min_filter: 0x2702, mag_filter: LINEAR, wrap_s: REPEAT, wrap_t: REPEAT }
    }

    fn complete(&self) -> bool { self.faces.iter().all(|f| f.width > 0 && f.height > 0) }

    fn fetch(l: &Level, x: i32, y: i32) -> [f32; 4] {
        let p = l.px[(y as u32 * l.width + x as u32) as usize];
        [(p & 0xFF) as f32 / 255.0, ((p >> 8) & 0xFF) as f32 / 255.0, ((p >> 16) & 0xFF) as f32 / 255.0, (p >> 24) as f32 / 255.0]
    }

    fn wrap(mode: u32, i: i32, n: i32) -> i32 {
        match mode {
            CLAMP_TO_EDGE => i.clamp(0, n - 1),
            MIRRORED_REPEAT => {
                let p = i.rem_euclid(2 * n);
                if p < n { p } else { 2 * n - 1 - p }
            }
            _ => i.rem_euclid(n),
        }
    }

    fn sample_level(&self, l: &Level, s: f32, t: f32) -> [f32; 4] {
        let (w, h) = (l.width as i32, l.height as i32);
        if self.mag_filter == NEAREST {
            let x = Self::wrap(self.wrap_s, libm::floorf(s * w as f32) as i32, w);
            let y = Self::wrap(self.wrap_t, libm::floorf(t * h as f32) as i32, h);
            return Self::fetch(l, x, y);
        }
        let (u, v) = (s * w as f32 - 0.5, t * h as f32 - 0.5);
        let (x0, y0) = (libm::floorf(u), libm::floorf(v));
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let (xa, xb) = (Self::wrap(self.wrap_s, x0, w), Self::wrap(self.wrap_s, x0 + 1, w));
        let (ya, yb) = (Self::wrap(self.wrap_t, y0, h), Self::wrap(self.wrap_t, y0 + 1, h));
        let (a, b, c, d) = (Self::fetch(l, xa, ya), Self::fetch(l, xb, ya), Self::fetch(l, xa, yb), Self::fetch(l, xb, yb));
        let mut o = [0f32; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bot = c[i] + (d[i] - c[i]) * fx;
            o[i] = top + (bot - top) * fy;
        }
        o
    }

    pub fn sample(&self, c: [f32; 4], depth_ref: Option<f32>) -> [f32; 4] {
        if !self.complete() { return [0.0, 0.0, 0.0, 1.0]; }
        let px = if self.target == TEXTURE_CUBE_MAP {
            let (x, y, z) = (c[0], c[1], c[2]);
            let (ax, ay, az) = (libm::fabsf(x), libm::fabsf(y), libm::fabsf(z));
            let (face, sc, tc, ma) = if ax >= ay && ax >= az {
                if x > 0.0 { (0, -z, -y, ax) } else { (1, z, -y, ax) }
            } else if ay >= az {
                if y > 0.0 { (2, x, z, ay) } else { (3, x, -z, ay) }
            } else if z > 0.0 { (4, x, -y, az) } else { (5, -x, -y, az) };
            let ma = if ma == 0.0 { 1.0 } else { ma };
            let mut t = self.clone_params();
            t.wrap_s = CLAMP_TO_EDGE;
            t.wrap_t = CLAMP_TO_EDGE;
            t.sample_level(&self.faces[face], (sc / ma + 1.0) * 0.5, (tc / ma + 1.0) * 0.5)
        } else {
            self.sample_level(&self.faces[0], c[0], c[1])
        };
        match depth_ref {
            Some(r) => { let v = if r <= px[0] { 1.0 } else { 0.0 }; [v, v, v, 1.0] }
            None => px,
        }
    }

    fn clone_params(&self) -> Texture {
        Texture { target: self.target, faces: Vec::new(), min_filter: self.min_filter, mag_filter: self.mag_filter, wrap_s: self.wrap_s, wrap_t: self.wrap_t }
    }
}

/// A shader object: its stage, source and last compile result.
pub struct ShaderObj {
    pub stage: Stage,
    pub source: String,
    pub compiled: Option<Shader>,
}

#[derive(Default)]
pub struct ProgramObj {
    pub shaders: Vec<u32>,
    pub bindings: BTreeMap<String, u32>,
    pub linked: Option<Program>,
    pub log: String,
    /// Uniform locations handed out: uniform index and element.
    pub locations: Vec<(usize, u32)>,
}

/// A WebGL drawing buffer and its state.
pub struct Context {
    pub width: u32,
    pub height: u32,
    pub color: Vec<u32>,
    pub depth: Vec<f32>,
    pub buffers: BTreeMap<u32, Vec<u8>>,
    pub textures: BTreeMap<u32, Texture>,
    pub shaders: BTreeMap<u32, ShaderObj>,
    pub programs: BTreeMap<u32, ProgramObj>,
    pub vaos: BTreeMap<u32, Vao>,
    pub default_vao: Vao,
    pub vao: u32,
    pub current: u32,
    pub array_buffer: u32,
    pub generic: [[f32; 4]; MAX_ATTRIBS],
    pub units: [[u32; 2]; MAX_UNITS],
    pub active_unit: usize,
    pub fixed: Fixed,
    pub scissor_box: [i32; 4],
    pub scissor_test: bool,
    pub polygon_offset: [f32; 2],
    pub polygon_offset_fill: bool,
    pub clear_color: [f32; 4],
    pub clear_depth: f32,
    pub unpack_alignment: u32,
    pub flip_y: bool,
    pub premultiply: bool,
    pub error: u32,
    /// Bumped by every draw or clear, so the page knows to show the new frame.
    pub frame: u64,
}

impl Context {
    pub fn new(width: u32, height: u32) -> Self {
        let (w, h) = (width.clamp(1, MAX_SIZE), height.clamp(1, MAX_SIZE));
        Context {
            width: w, height: h, color: vec![0; (w * h) as usize], depth: vec![1.0; (w * h) as usize],
            buffers: BTreeMap::new(), textures: BTreeMap::new(), shaders: BTreeMap::new(), programs: BTreeMap::new(),
            vaos: BTreeMap::new(), default_vao: Vao::default(), vao: 0, current: 0, array_buffer: 0,
            generic: [[0.0, 0.0, 0.0, 1.0]; MAX_ATTRIBS], units: [[0; 2]; MAX_UNITS], active_unit: 0,
            fixed: Fixed {
                viewport: [0, 0, w as i32, h as i32], scissor: None, depth_test: false, depth_func: rg::LESS, depth_mask: true,
                depth_range: [0.0, 1.0], blend: false, blend_src: [rg::ONE; 2], blend_dst: [rg::ZERO; 2], blend_eq: [rg::FUNC_ADD; 2],
                blend_color: [0.0; 4], color_mask: [true; 4], cull: false, cull_face: rg::BACK, front_ccw: true, polygon_offset: None, zero_one: false,
            },
            scissor_box: [0, 0, w as i32, h as i32], scissor_test: false, polygon_offset: [0.0; 2], polygon_offset_fill: false,
            clear_color: [0.0; 4], clear_depth: 1.0, unpack_alignment: 4, flip_y: false, premultiply: false, error: NO_ERROR, frame: 0,
        }
    }

    fn err(&mut self, e: u32) { if self.error == NO_ERROR { self.error = e; } }

    pub fn get_error(&mut self) -> u32 { core::mem::replace(&mut self.error, NO_ERROR) }

    /// Resizes (and clears) the drawing buffer, as setting canvas.width/height does.
    pub fn resize(&mut self, width: u32, height: u32) {
        let (w, h) = (width.clamp(1, MAX_SIZE), height.clamp(1, MAX_SIZE));
        self.width = w;
        self.height = h;
        self.color = vec![0; (w * h) as usize];
        self.depth = vec![1.0; (w * h) as usize];
        self.frame += 1;
    }

    fn vao_mut(&mut self) -> &mut Vao {
        if self.vao == 0 { &mut self.default_vao } else { self.vaos.entry(self.vao).or_default() }
    }
    fn vao_ref(&self) -> &Vao { if self.vao == 0 { &self.default_vao } else { self.vaos.get(&self.vao).unwrap_or(&self.default_vao) } }

    // ---- State ----
    pub fn enable(&mut self, cap: u32, on: bool) {
        match cap {
            CULL_FACE => self.fixed.cull = on,
            BLEND => self.fixed.blend = on,
            DEPTH_TEST => self.fixed.depth_test = on,
            SCISSOR_TEST => self.scissor_test = on,
            POLYGON_OFFSET_FILL => self.polygon_offset_fill = on,
            _ => {}
        }
    }
    pub fn is_enabled(&self, cap: u32) -> bool {
        match cap {
            CULL_FACE => self.fixed.cull,
            BLEND => self.fixed.blend,
            DEPTH_TEST => self.fixed.depth_test,
            SCISSOR_TEST => self.scissor_test,
            POLYGON_OFFSET_FILL => self.polygon_offset_fill,
            _ => false,
        }
    }
    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) { self.fixed.viewport = [x, y, w.max(0), h.max(0)]; }
    pub fn scissor(&mut self, x: i32, y: i32, w: i32, h: i32) { self.scissor_box = [x, y, w.max(0), h.max(0)]; }
    pub fn depth_func(&mut self, f: u32) { self.fixed.depth_func = f; }
    pub fn depth_mask(&mut self, m: bool) { self.fixed.depth_mask = m; }
    pub fn depth_range(&mut self, n: f32, f: f32) { self.fixed.depth_range = [n.clamp(0.0, 1.0), f.clamp(0.0, 1.0)]; }
    pub fn color_mask(&mut self, r: bool, g: bool, b: bool, a: bool) { self.fixed.color_mask = [r, g, b, a]; }
    pub fn cull_face(&mut self, m: u32) { self.fixed.cull_face = m; }
    pub fn front_face(&mut self, m: u32) { self.fixed.front_ccw = m != CW; }
    pub fn blend_func(&mut self, src_rgb: u32, dst_rgb: u32, src_a: u32, dst_a: u32) {
        self.fixed.blend_src = [src_rgb, src_a];
        self.fixed.blend_dst = [dst_rgb, dst_a];
    }
    pub fn blend_equation(&mut self, rgb: u32, a: u32) { self.fixed.blend_eq = [rgb, a]; }
    pub fn blend_color(&mut self, c: [f32; 4]) { self.fixed.blend_color = c; }
    pub fn polygon_offset(&mut self, factor: f32, units: f32) { self.polygon_offset = [factor, units]; }
    pub fn clear_color(&mut self, c: [f32; 4]) { self.clear_color = c; }
    pub fn clear_depth(&mut self, d: f32) { self.clear_depth = d.clamp(0.0, 1.0); }
    pub fn pixel_store(&mut self, pname: u32, v: i32) {
        match pname {
            UNPACK_ALIGNMENT => self.unpack_alignment = (v as u32).clamp(1, 8),
            UNPACK_FLIP_Y_WEBGL => self.flip_y = v != 0,
            UNPACK_PREMULTIPLY_ALPHA_WEBGL => self.premultiply = v != 0,
            _ => {}
        }
    }

    pub fn clear(&mut self, mask: u32) {
        let (x0, y0, x1, y1) = self.clip_rect();
        if mask & COLOR_BUFFER_BIT != 0 {
            let c = raster::pack(self.clear_color);
            let cm = self.fixed.color_mask;
            let mut m = 0u32;
            for ch in 0..4 { if cm[ch] { m |= 0xFF << (ch * 8); } }
            for y in y0..y1 { for x in x0..x1 {
                let i = (y as u32 * self.width + x as u32) as usize;
                self.color[i] = (c & m) | (self.color[i] & !m);
            } }
        }
        if mask & DEPTH_BUFFER_BIT != 0 && self.fixed.depth_mask {
            for y in y0..y1 { for x in x0..x1 { self.depth[(y as u32 * self.width + x as u32) as usize] = self.clear_depth; } }
        }
        self.frame += 1;
    }

    fn clip_rect(&self) -> (i32, i32, i32, i32) {
        let (mut x0, mut y0, mut x1, mut y1) = (0, 0, self.width as i32, self.height as i32);
        if self.scissor_test {
            let s = self.scissor_box;
            x0 = x0.max(s[0]); y0 = y0.max(s[1]); x1 = x1.min(s[0] + s[2]); y1 = y1.min(s[1] + s[3]);
        }
        (x0, y0, x1.max(x0), y1.max(y0))
    }

    // ---- Buffers ----
    pub fn bind_buffer(&mut self, target: u32, id: u32) {
        match target {
            ARRAY_BUFFER => self.array_buffer = id,
            ELEMENT_ARRAY_BUFFER => self.vao_mut().element = id,
            _ => self.err(INVALID_ENUM),
        }
        if id != 0 { self.buffers.entry(id).or_default(); }
    }
    fn bound(&self, target: u32) -> u32 { if target == ELEMENT_ARRAY_BUFFER { self.vao_ref().element } else { self.array_buffer } }
    pub fn buffer_data(&mut self, target: u32, data: &[u8]) {
        let id = self.bound(target);
        if id == 0 { return self.err(INVALID_OPERATION); }
        self.buffers.insert(id, data.to_vec());
    }
    pub fn buffer_sub_data(&mut self, target: u32, offset: usize, data: &[u8]) {
        let id = self.bound(target);
        match self.buffers.get_mut(&id) {
            Some(b) if offset + data.len() <= b.len() => b[offset..offset + data.len()].copy_from_slice(data),
            _ => self.err(INVALID_VALUE),
        }
    }
    pub fn delete_buffer(&mut self, id: u32) { self.buffers.remove(&id); }

    // ---- Vertex arrays ----
    pub fn bind_vertex_array(&mut self, id: u32) { self.vao = id; if id != 0 { self.vaos.entry(id).or_default(); } }
    pub fn delete_vertex_array(&mut self, id: u32) { self.vaos.remove(&id); if self.vao == id { self.vao = 0; } }
    pub fn enable_attrib(&mut self, i: u32, on: bool) {
        if (i as usize) < MAX_ATTRIBS { self.vao_mut().attribs[i as usize].enabled = on; } else { self.err(INVALID_VALUE); }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn attrib_pointer(&mut self, i: u32, size: u32, ty: u32, normalized: bool, stride: u32, offset: u32) {
        if i as usize >= MAX_ATTRIBS { return self.err(INVALID_VALUE); }
        let buffer = self.array_buffer;
        let a = &mut self.vao_mut().attribs[i as usize];
        *a = Attrib { enabled: a.enabled, buffer, size: size.clamp(1, 4), ty, normalized, stride, offset, divisor: a.divisor, integer: false };
    }
    /// An integer attribute: components keep their integer values.
    pub fn attrib_ipointer(&mut self, i: u32, size: u32, ty: u32, stride: u32, offset: u32) {
        self.attrib_pointer(i, size, ty, false, stride, offset);
        if (i as usize) < MAX_ATTRIBS { self.vao_mut().attribs[i as usize].integer = true; }
    }
    pub fn attrib_divisor(&mut self, i: u32, d: u32) { if (i as usize) < MAX_ATTRIBS { self.vao_mut().attribs[i as usize].divisor = d; } }
    pub fn vertex_attrib(&mut self, i: u32, v: [f32; 4]) { if (i as usize) < MAX_ATTRIBS { self.generic[i as usize] = v; } }

    // ---- Textures ----
    pub fn active_texture(&mut self, unit: u32) {
        let u = unit.wrapping_sub(TEXTURE0) as usize;
        if u < MAX_UNITS { self.active_unit = u; } else { self.err(INVALID_ENUM); }
    }
    pub fn bind_texture(&mut self, target: u32, id: u32) {
        let slot = if target == TEXTURE_CUBE_MAP { 1 } else { 0 };
        if id != 0 {
            let t = self.textures.entry(id).or_insert_with(|| Texture::new(target));
            if t.target != target { return self.err(INVALID_OPERATION); }
        }
        self.units[self.active_unit][slot] = id;
    }
    pub fn delete_texture(&mut self, id: u32) {
        self.textures.remove(&id);
        for u in self.units.iter_mut() { for s in u.iter_mut() { if *s == id { *s = 0; } } }
    }
    fn bound_texture(&mut self, target: u32) -> Option<(&mut Texture, usize)> {
        let (slot, face) = if target == TEXTURE_2D { (0, 0) } else if target == TEXTURE_CUBE_MAP { (1, 0) } else {
            let f = target.wrapping_sub(TEXTURE_CUBE_MAP_POSITIVE_X);
            if f < 6 { (1, f as usize) } else { return None }
        };
        let id = self.units[self.active_unit][slot];
        self.textures.get_mut(&id).map(|t| (t, face))
    }
    pub fn tex_parameter(&mut self, target: u32, pname: u32, v: u32) {
        let Some((t, _)) = self.bound_texture(target) else { return self.err(INVALID_OPERATION) };
        match pname {
            TEXTURE_MIN_FILTER => t.min_filter = v,
            TEXTURE_MAG_FILTER => t.mag_filter = v,
            TEXTURE_WRAP_S => t.wrap_s = v,
            TEXTURE_WRAP_T => t.wrap_t = v,
            _ => {}
        }
    }

    /// Converts client pixels in `format`/`ty` to RGBA8 rows, honouring the unpack state.
    pub fn convert(&self, width: u32, height: u32, format: u32, ty: u32, data: &[u8]) -> Option<Vec<u32>> {
        let comps = match format { RGBA => 4, RGB => 3, LUMINANCE_ALPHA | RG => 2, _ => 1 };
        let bpp = match ty { FLOAT => 4 * comps, HALF_FLOAT | HALF_FLOAT_OES => 2 * comps, UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => 2, _ => comps };
        let row = width as usize * bpp;
        let align = self.unpack_alignment as usize;
        let stride = row.div_ceil(align) * align;
        if height > 0 && data.len() < stride * (height as usize - 1) + row { return None; }
        let mut out = vec![0u32; (width * height) as usize];
        for y in 0..height as usize {
            let src_y = if self.flip_y { height as usize - 1 - y } else { y };
            let r = &data[src_y * stride..src_y * stride + row];
            for x in 0..width as usize {
                let mut c: [f32; 4];
                let comp = |i: usize| -> f32 {
                    match ty {
                        FLOAT => f32::from_le_bytes([r[x * bpp + i * 4], r[x * bpp + i * 4 + 1], r[x * bpp + i * 4 + 2], r[x * bpp + i * 4 + 3]]),
                        HALF_FLOAT | HALF_FLOAT_OES => half(u16::from_le_bytes([r[x * bpp + i * 2], r[x * bpp + i * 2 + 1]])),
                        _ => r[x * bpp + i] as f32 / 255.0,
                    }
                };
                match ty {
                    UNSIGNED_SHORT_5_6_5 | UNSIGNED_SHORT_4_4_4_4 | UNSIGNED_SHORT_5_5_5_1 => {
                        let v = u16::from_le_bytes([r[x * 2], r[x * 2 + 1]]) as u32;
                        c = match ty {
                            UNSIGNED_SHORT_5_6_5 => [(v >> 11) as f32 / 31.0, ((v >> 5) & 63) as f32 / 63.0, (v & 31) as f32 / 31.0, 1.0],
                            UNSIGNED_SHORT_4_4_4_4 => [(v >> 12) as f32 / 15.0, ((v >> 8) & 15) as f32 / 15.0, ((v >> 4) & 15) as f32 / 15.0, (v & 15) as f32 / 15.0],
                            _ => [(v >> 11) as f32 / 31.0, ((v >> 6) & 31) as f32 / 31.0, ((v >> 1) & 31) as f32 / 31.0, (v & 1) as f32],
                        };
                    }
                    _ => match format {
                        RGBA => c = [comp(0), comp(1), comp(2), comp(3)],
                        RGB => c = [comp(0), comp(1), comp(2), 1.0],
                        LUMINANCE => { let l = comp(0); c = [l, l, l, 1.0]; }
                        LUMINANCE_ALPHA => { let l = comp(0); c = [l, l, l, comp(1)]; }
                        ALPHA => c = [0.0, 0.0, 0.0, comp(0)],
                        RED => c = [comp(0), 0.0, 0.0, 1.0],
                        RG => c = [comp(0), comp(1), 0.0, 1.0],
                        _ => c = [comp(0), comp(1), comp(2), comp(3)],
                    },
                }
                if self.premultiply { for i in 0..3 { c[i] *= c[3]; } }
                out[y * width as usize + x] = raster::pack(c);
            }
        }
        Some(out)
    }

    /// texImage2D with client data (`None` allocates zeroed storage). Only level 0 is kept.
    #[allow(clippy::too_many_arguments)]
    pub fn tex_image(&mut self, target: u32, level: i32, width: u32, height: u32, format: u32, ty: u32, data: Option<&[u8]>) {
        if level != 0 { return; }
        if width > MAX_SIZE || height > MAX_SIZE { return self.err(INVALID_VALUE); }
        let px = match data {
            Some(d) => match self.convert(width, height, format, ty, d) { Some(p) => p, None => return self.err(INVALID_OPERATION) },
            None => vec![0; (width * height) as usize],
        };
        self.tex_image_rgba(target, width, height, px);
    }

    /// texImage2D from RGBA8 pixels already decoded (pictures, other canvases), top row
    /// first as images are; flip-y and premultiply apply.
    pub fn tex_image_rgba(&mut self, target: u32, width: u32, height: u32, px: Vec<u32>) {
        let Some((t, face)) = self.bound_texture(target) else { return self.err(INVALID_OPERATION) };
        t.faces[face] = Level { width, height, px };
    }

    /// Uploads a decoded picture (0xAARRGGBB, top row first, not premultiplied).
    pub fn tex_image_argb(&mut self, target: u32, width: u32, height: u32, argb: &[u32]) {
        let mut px = vec![0u32; (width * height) as usize];
        for y in 0..height as usize {
            // DOM pictures upload top row first unless UNPACK_FLIP_Y_WEBGL.
            let src_y = if self.flip_y { height as usize - 1 - y } else { y };
            for x in 0..width as usize {
                let p = argb[src_y * width as usize + x];
                let (a, r, g, b) = (p >> 24, (p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
                let (r, g, b) = if self.premultiply { (r * a / 255, g * a / 255, b * a / 255) } else { (r, g, b) };
                px[y * width as usize + x] = r | g << 8 | b << 16 | a << 24;
            }
        }
        self.tex_image_rgba(target, width, height, px);
    }

    /// Uploads a picture whose pixels arrived late into texture `tex`, with the unpack
    /// state of the moment it was asked for; bindings and unpack state are kept.
    #[allow(clippy::too_many_arguments)]
    pub fn upload_picture(&mut self, target: u32, tex: u32, width: u32, height: u32, argb: &[u32], flip_y: bool, premultiply: bool) {
        if !self.textures.contains_key(&tex) || argb.len() < (width * height) as usize { return; }
        let slot = if target == TEXTURE_2D { 0 } else { 1 };
        let saved = (self.units[self.active_unit][slot], self.flip_y, self.premultiply);
        self.units[self.active_unit][slot] = tex;
        self.flip_y = flip_y;
        self.premultiply = premultiply;
        self.tex_image_argb(target, width, height, argb);
        self.units[self.active_unit][slot] = saved.0;
        self.flip_y = saved.1;
        self.premultiply = saved.2;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn tex_sub_image(&mut self, target: u32, level: i32, x: u32, y: u32, width: u32, height: u32, format: u32, ty: u32, data: &[u8]) {
        if level != 0 { return; }
        let Some(px) = self.convert(width, height, format, ty, data) else { return self.err(INVALID_OPERATION) };
        let Some((t, face)) = self.bound_texture(target) else { return self.err(INVALID_OPERATION) };
        let l = &mut t.faces[face];
        if x + width > l.width || y + height > l.height { return self.err(INVALID_VALUE); }
        for row in 0..height {
            for col in 0..width { l.px[((y + row) * l.width + x + col) as usize] = px[(row * width + col) as usize]; }
        }
    }

    // ---- Shaders and programs ----
    pub fn create_shader(&mut self, id: u32, kind: u32) {
        let stage = if kind == VERTEX_SHADER { Stage::Vertex } else { Stage::Fragment };
        self.shaders.insert(id, ShaderObj { stage, source: String::new(), compiled: None });
    }
    pub fn shader_source(&mut self, id: u32, src: &str) {
        if let Some(s) = self.shaders.get_mut(&id) { s.source = src.into(); }
    }
    pub fn compile_shader(&mut self, id: u32) {
        if let Some(s) = self.shaders.get_mut(&id) { s.compiled = Some(Shader::compile(s.stage, &s.source)); }
    }
    pub fn shader_status(&self, id: u32) -> (bool, String) {
        match self.shaders.get(&id).and_then(|s| s.compiled.as_ref()) {
            Some(c) => (c.ok, c.log.clone()),
            None => (false, String::new()),
        }
    }
    pub fn create_program(&mut self, id: u32) { self.programs.insert(id, ProgramObj::default()); }
    pub fn delete_program(&mut self, id: u32) { self.programs.remove(&id); }
    pub fn delete_shader(&mut self, id: u32) { self.shaders.remove(&id); }
    pub fn attach_shader(&mut self, p: u32, s: u32) { if let Some(p) = self.programs.get_mut(&p) { if !p.shaders.contains(&s) { p.shaders.push(s); } } }
    pub fn bind_attrib_location(&mut self, p: u32, loc: u32, name: &str) {
        if let Some(p) = self.programs.get_mut(&p) { p.bindings.insert(name.into(), loc); }
    }
    pub fn link_program(&mut self, id: u32) {
        let Some(p) = self.programs.get(&id) else { return };
        let mut vs = None;
        let mut fs = None;
        for s in &p.shaders {
            if let Some(so) = self.shaders.get(s) {
                let c = Shader::compile(so.stage, &so.source);
                match so.stage { Stage::Vertex => vs = Some(c), Stage::Fragment => fs = Some(c) }
            }
        }
        let res = match (vs, fs) {
            (Some(v), Some(f)) => Program::link(&v, &f, &p.bindings),
            _ => Err("ERROR: a program needs a vertex and a fragment shader".into()),
        };
        let p = self.programs.get_mut(&id).unwrap();
        p.locations.clear();
        match res {
            Ok(prog) => { p.linked = Some(prog); p.log.clear(); }
            Err(e) => { p.linked = None; p.log = e; }
        }
    }
    pub fn use_program(&mut self, id: u32) { self.current = id; }

    /// WebGPU: a render pipeline's program from WGSL modules, as program `id`.
    pub fn gpu_program(&mut self, id: u32, vs_code: &str, vs_entry: &str, fs_code: &str, fs_entry: &str) {
        let (linked, log) = match Program::from_wgsl(vs_code, vs_entry, fs_code, fs_entry) {
            Ok(p) => (Some(p), String::new()),
            Err(e) => (None, e),
        };
        self.programs.insert(id, ProgramObj { linked, log, ..Default::default() });
    }

    /// WebGPU: binds `size` bytes of buffer `buf` from `offset` to a buffer binding.
    pub fn gpu_bind_buffer(&mut self, prog: u32, group: u32, binding: u32, buf: u32, offset: usize, size: usize) {
        let bytes: Vec<u8> = match self.buffers.get(&buf) {
            Some(b) => { let end = offset.saturating_add(size).min(b.len()); b.get(offset.min(end)..end).unwrap_or(&[]).to_vec() }
            None => return,
        };
        if let Some(p) = self.programs.get_mut(&prog).and_then(|p| p.linked.as_mut()) { p.bind_buffer(group, binding, &bytes); }
    }

    /// WebGPU: a texture binding samples the texture bound to `unit`.
    pub fn gpu_bind_texture(&mut self, prog: u32, group: u32, binding: u32, unit: u32) {
        let Some(p) = self.programs.get_mut(&prog).and_then(|p| p.linked.as_mut()) else { return };
        let Some(b) = p.gpu.iter().find(|b| b.group == group && b.binding == binding) else { return };
        if let gpu::BindKind::Texture(i) = b.kind { p.samplers[i].units = alloc::vec![unit as i32]; }
    }

    /// WebGPU: a sampler binding sets the filters and wrapping of the textures it samples.
    pub fn gpu_bind_sampler(&mut self, prog: u32, group: u32, binding: u32, filters: [u32; 4]) {
        let Some(p) = self.programs.get(&prog).and_then(|p| p.linked.as_ref()) else { return };
        let Some(b) = p.gpu.iter().find(|b| b.group == group && b.binding == binding) else { return };
        let gpu::BindKind::Sampler(texs) = &b.kind else { return };
        let mut ids = Vec::new();
        for &i in texs {
            let s = &p.samplers[i];
            let unit = s.units.first().copied().unwrap_or(0).clamp(0, MAX_UNITS as i32 - 1) as usize;
            ids.push(self.units[unit][if s.kind == SamplerKind::Cube { 1 } else { 0 }]);
        }
        for id in ids {
            if let Some(t) = self.textures.get_mut(&id) {
                [t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t] = filters;
            }
        }
    }
    pub fn program(&self, id: u32) -> Option<&Program> { self.programs.get(&id).and_then(|p| p.linked.as_ref()) }

    /// A uniform location (an index for this program), or `None`.
    pub fn uniform_location(&mut self, prog: u32, name: &str) -> Option<u32> {
        let p = self.programs.get_mut(&prog)?;
        let lp = p.linked.as_ref()?;
        let name = name.strip_suffix("[0]").filter(|n| lp.locate(n).is_some_and(|(u, _)| lp.uniforms[u].size > 1 || lp.uniforms[u].name.ends_with(']'))).unwrap_or(name);
        let loc = lp.locate(name)?;
        if let Some(i) = p.locations.iter().position(|l| *l == loc) { return Some(i as u32); }
        p.locations.push(loc);
        Some(p.locations.len() as u32 - 1)
    }

    /// Sets a uniform of the current program from float or integer values.
    pub fn uniform(&mut self, loc: u32, floats: Option<&[f32]>, ints: Option<&[i32]>) {
        let Some(p) = self.programs.get_mut(&self.current) else { return self.err(INVALID_OPERATION) };
        let Some(&(u, elem)) = p.locations.get(loc as usize) else { return };
        let Some(prog) = p.linked.as_mut() else { return };
        let kind = prog.uniforms[u].vs.or(prog.uniforms[u].fs).map(|s| s.kind);
        let bits: Vec<u32> = match (floats, ints, kind) {
            (Some(f), _, Some(exec::K::B)) => f.iter().map(|v| (*v != 0.0) as u32).collect(),
            (_, Some(i), Some(exec::K::B)) => i.iter().map(|v| (*v != 0) as u32).collect(),
            (Some(f), _, Some(exec::K::I | exec::K::U)) => f.iter().map(|v| *v as i32 as u32).collect(),
            (Some(f), _, _) => f.iter().map(|v| v.to_bits()).collect(),
            (_, Some(i), Some(exec::K::F)) => i.iter().map(|v| (*v as f32).to_bits()).collect(),
            (_, Some(i), _) => i.iter().map(|v| *v as u32).collect(),
            _ => return,
        };
        prog.set(u, elem, &bits);
    }

    // ---- Drawing ----
    fn fetch(&self, a: &Attrib, i: u32, out: &mut [u32], lanes: usize) {
        let Some(buf) = self.buffers.get(&a.buffer) else { return };
        let csize = match a.ty { BYTE | UNSIGNED_BYTE => 1, SHORT | UNSIGNED_SHORT | HALF_FLOAT | HALF_FLOAT_OES => 2, _ => 4 };
        let stride = if a.stride == 0 { a.size * csize } else { a.stride } as usize;
        let base = a.offset as usize + i as usize * stride;
        if a.integer {
            let mut v = [0u32, 0, 0, 1];
            for c in 0..a.size as usize {
                let o = base + c * csize as usize;
                if o + csize as usize > buf.len() { return; }
                let b = &buf[o..o + csize as usize];
                v[c] = match a.ty {
                    UNSIGNED_BYTE => b[0] as u32,
                    BYTE => b[0] as i8 as i32 as u32,
                    UNSIGNED_SHORT => u16::from_le_bytes([b[0], b[1]]) as u32,
                    SHORT => i16::from_le_bytes([b[0], b[1]]) as i32 as u32,
                    _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                };
            }
            for (j, o) in out.iter_mut().enumerate().take(lanes.min(4)) { *o = v[j]; }
            return;
        }
        let mut v = [0f32, 0.0, 0.0, 1.0];
        for c in 0..a.size as usize {
            let o = base + c * csize as usize;
            if o + csize as usize > buf.len() { return; }
            let b = &buf[o..o + csize as usize];
            v[c] = match a.ty {
                FLOAT => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                HALF_FLOAT | HALF_FLOAT_OES => half(u16::from_le_bytes([b[0], b[1]])),
                UNSIGNED_BYTE => if a.normalized { b[0] as f32 / 255.0 } else { b[0] as f32 },
                BYTE => if a.normalized { (b[0] as i8 as f32 / 127.0).max(-1.0) } else { b[0] as i8 as f32 },
                UNSIGNED_SHORT => { let x = u16::from_le_bytes([b[0], b[1]]); if a.normalized { x as f32 / 65535.0 } else { x as f32 } }
                SHORT => { let x = i16::from_le_bytes([b[0], b[1]]); if a.normalized { (x as f32 / 32767.0).max(-1.0) } else { x as f32 } }
                UNSIGNED_INT => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                INT => i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                _ => 0.0,
            };
        }
        for (j, o) in out.iter_mut().enumerate().take(lanes.min(4)) { *o = v[j].to_bits(); }
    }

    pub fn draw_arrays(&mut self, mode: u32, first: i32, count: i32) {
        if first < 0 || count < 0 { return self.err(INVALID_VALUE); }
        let idx: Vec<u32> = (first as u32..(first + count) as u32).collect();
        self.draw(mode, &idx);
    }

    pub fn draw_elements(&mut self, mode: u32, count: i32, ty: u32, offset: u32) {
        if count < 0 { return self.err(INVALID_VALUE); }
        let Some(buf) = self.buffers.get(&self.vao_ref().element) else { return self.err(INVALID_OPERATION) };
        let size = match ty { UNSIGNED_BYTE => 1, UNSIGNED_SHORT => 2, UNSIGNED_INT => 4, _ => return self.err(INVALID_ENUM) };
        let mut idx = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let o = offset as usize + i * size;
            if o + size > buf.len() { return self.err(INVALID_OPERATION); }
            idx.push(match size { 1 => buf[o] as u32, 2 => u16::from_le_bytes([buf[o], buf[o + 1]]) as u32, _ => u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]) });
        }
        self.draw(mode, &idx);
    }

    fn draw(&mut self, mode: u32, idx: &[u32]) {
        if idx.is_empty() { return; }
        let (mut color, mut depth) = (core::mem::take(&mut self.color), core::mem::take(&mut self.depth));
        let res = self.draw_into(mode, idx, &mut color, &mut depth);
        self.color = color;
        self.depth = depth;
        match res { Ok(()) => self.frame += 1, Err(e) => self.err(e) }
    }

    fn draw_into(&self, mode: u32, idx: &[u32], color: &mut [u32], depth: &mut [f32]) -> Result<(), u32> {
        let Some(prog) = self.programs.get(&self.current).and_then(|p| p.linked.as_ref()) else { return Err(INVALID_OPERATION) };
        // Shade each distinct vertex once.
        let (lo, hi) = (*idx.iter().min().unwrap(), *idx.iter().max().unwrap());
        if hi - lo > 1 << 24 { return Err(INVALID_OPERATION); }
        let vao = self.vao_ref().clone();
        let tex_vs = DrawTextures::new(self, prog, true);
        let mut vm = prog.vs.machine();
        let mut cache: Vec<Option<u32>> = vec![None; (hi - lo + 1) as usize];
        let mut verts: Vec<Vert> = Vec::new();
        let mut local = Vec::with_capacity(idx.len());
        for &i in idx {
            let slot = &mut cache[(i - lo) as usize];
            let vi = match slot {
                Some(v) => *v,
                None => {
                    prog.vs.run(&mut vm, &prog.vs_uniforms, &tex_vs, &mut |io, lanes| match io {
                        Io::Location(l) => {
                            let n = lanes.len();
                            // Matrices span consecutive locations, one column each.
                            let cols = if n > 4 { n / if n == 9 { 3 } else if n == 4 { 2 } else { 4 } } else { 1 };
                            let per = n / cols;
                            for c in 0..cols {
                                let loc = (l as usize + c).min(MAX_ATTRIBS - 1);
                                let a = &vao.attribs[loc];
                                let part = &mut lanes[c * per..(c + 1) * per];
                                if a.enabled { self.fetch(a, if a.divisor > 0 { 0 } else { i }, part, per) } else {
                                    for (j, p) in part.iter_mut().enumerate() { *p = self.generic[loc][j.min(3)].to_bits(); }
                                }
                            }
                        }
                        Io::VertexIndex => lanes[0] = i,
                        _ => {}
                    });
                    let out = &vm.out;
                    let f = |o: usize| f32::from_bits(out[o]);
                    let p = prog.position as usize;
                    let mut vary = Vec::with_capacity(prog.varying_lanes as usize);
                    for &(vo, _, n) in &prog.links { for k in 0..n as usize { vary.push(f(vo as usize + k)); } }
                    verts.push(Vert { pos: [f(p), f(p + 1), f(p + 2), f(p + 3)], point_size: prog.point_size.map_or(1.0, |o| f(o as usize)), vary });
                    *slot = Some(verts.len() as u32 - 1);
                    verts.len() as u32 - 1
                }
            };
            local.push(vi);
        }
        // Fragment inputs: where each comes from in the packed varyings.
        let mut packed = Vec::new();
        let mut at = 0;
        for &(_, fo, n) in &prog.links { packed.push((fo, at, n)); at += n; }
        let fin: Vec<(Io, u32, Option<u32>)> = prog.fs.inputs.iter().map(|(io, off, n)| (*io, *n, packed.iter().find(|p| p.0 == *off).map(|p| p.1))).collect();
        let tex_fs = DrawTextures::new(self, prog, false);
        let mut fixed = self.fixed;
        fixed.scissor = if self.scissor_test { Some(self.scissor_box) } else { None };
        fixed.polygon_offset = if self.polygon_offset_fill { Some(self.polygon_offset) } else { None };
        fixed.zero_one = prog.zero_one;
        let mut shader = FragShader { prog, m: prog.fs.machine(), wide: None, tex: &tex_fs, fin };
        let mut target = Target { width: self.width as i32, height: self.height as i32, color, depth };
        raster::assemble(&mut target, &fixed, mode, &local, &verts, &mut shader);
        Ok(())
    }

    /// RGBA8 bytes of a rectangle, bottom row first (readPixels).
    pub fn read_pixels(&self, x: i32, y: i32, w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![0u8; (w * h * 4) as usize];
        for row in 0..h as i32 {
            for col in 0..w as i32 {
                let (px, py) = (x + col, y + row);
                if px < 0 || py < 0 || px >= self.width as i32 || py >= self.height as i32 { continue; }
                let p = self.color[(py as u32 * self.width + px as u32) as usize];
                let o = ((row as u32 * w + col as u32) * 4) as usize;
                out[o..o + 4].copy_from_slice(&p.to_le_bytes());
            }
        }
        out
    }

    /// The drawing buffer as a picture: 0xAARRGGBB, top row first, un-premultiplied.
    /// `alpha: false` contexts are shown opaque.
    pub fn present(&self, alpha: bool) -> Vec<u32> {
        let (w, h) = (self.width as usize, self.height as usize);
        let mut out = vec![0u32; w * h];
        for y in 0..h {
            let src = &self.color[(h - 1 - y) * w..(h - y) * w];
            for (x, &p) in src.iter().enumerate() {
                let (r, g, b, a) = (p & 0xFF, (p >> 8) & 0xFF, (p >> 16) & 0xFF, p >> 24);
                out[y * w + x] = if !alpha || a == 255 { 0xFF00_0000 | r << 16 | g << 8 | b } else if a == 0 { 0 } else {
                    let un = |c: u32| ((c * 255 + a / 2) / a).min(255);
                    a << 24 | un(r) << 16 | un(g) << 8 | un(b)
                };
            }
        }
        out
    }
}

fn half(h: u16) -> f32 {
    let (s, e, m) = ((h >> 15) as u32, ((h >> 10) & 0x1F) as u32, (h & 0x3FF) as u32);
    let bits = if e == 0 {
        if m == 0 { s << 31 } else {
            let mut e2 = 127 - 15 + 1;
            let mut m2 = m;
            while m2 & 0x400 == 0 { m2 <<= 1; e2 -= 1; }
            s << 31 | (e2 as u32) << 23 | (m2 & 0x3FF) << 13
        }
    } else if e == 31 { s << 31 | 0xFF << 23 | m << 13 } else { s << 31 | (e + 127 - 15) << 23 | m << 13 };
    f32::from_bits(bits)
}

/// Texture lookups for one stage of one draw: texture global -> bound texture.
struct DrawTextures<'a> {
    by_global: Vec<Option<&'a Texture>>,
}

impl<'a> DrawTextures<'a> {
    fn new(ctx: &'a Context, prog: &Program, vs: bool) -> Self {
        let mut by_global: Vec<Option<&Texture>> = Vec::new();
        for s in &prog.samplers {
            let globals = if vs { &s.vs_globals } else { &s.fs_globals };
            let slot = if s.kind == SamplerKind::Cube { 1 } else { 0 };
            for (i, g) in globals.iter().enumerate() {
                let unit = s.units.get(i).copied().unwrap_or(0).clamp(0, MAX_UNITS as i32 - 1) as usize;
                let id = ctx.units[unit][slot];
                let g = *g as usize;
                if by_global.len() <= g { by_global.resize(g + 1, None); }
                by_global[g] = ctx.textures.get(&id);
            }
        }
        DrawTextures { by_global }
    }
}

impl Textures for DrawTextures<'_> {
    fn sample(&self, global: u32, coord: [f32; 4], _lod: f32, depth_ref: Option<f32>) -> [f32; 4] {
        match self.by_global.get(global as usize).copied().flatten() {
            Some(t) => t.sample(coord, depth_ref),
            None => [0.0, 0.0, 0.0, 1.0],
        }
    }
}

struct FragShader<'a> {
    prog: &'a Program,
    m: exec::Machine,
    wide: Option<exec::WideMachine>,
    tex: &'a DrawTextures<'a>,
    fin: Vec<(Io, u32, Option<u32>)>,
}

impl FragShader<'_> {
    fn fill(fin: &[(Io, u32, Option<u32>)], frag: &Frag, vary: &[f32], io: Io, lanes: &mut [u32]) {
        match io {
            Io::Location(_) => {
                if let Some((_, n, Some(at))) = fin.iter().find(|(i, _, _)| *i == io) {
                    for k in 0..(*n as usize).min(lanes.len()) { lanes[k] = vary.get(*at as usize + k).copied().unwrap_or(0.0).to_bits(); }
                }
            }
            Io::Position => {
                let v = [frag.x, frag.y, frag.z, frag.inv_w];
                for (k, l) in lanes.iter_mut().enumerate().take(4) { *l = v[k].to_bits(); }
            }
            Io::FrontFacing => lanes[0] = frag.front as u32,
            Io::PointCoord => { lanes[0] = frag.point[0].to_bits(); if lanes.len() > 1 { lanes[1] = frag.point[1].to_bits(); } }
            _ => {}
        }
    }

    fn result(&self, out: impl Fn(u32) -> u32) -> ([f32; 4], Option<f32>) {
        let c = match self.prog.color {
            Some((o, n)) => {
                let mut c = [0f32, 0.0, 0.0, 1.0];
                for k in 0..(n as usize).min(4) { c[k] = f32::from_bits(out(o + k as u32)); }
                c
            }
            None => [0.0; 4],
        };
        let z = self.prog.frag_depth.map(|o| f32::from_bits(out(o)).clamp(0.0, 1.0));
        (c, z)
    }
}

impl Shade for FragShader<'_> {
    fn writes_depth(&self) -> bool { self.prog.frag_depth.is_some() }
    fn batch(&self) -> usize { exec::WIDE }
    fn shade(&mut self, frag: &Frag, vary: &[f32]) -> Option<([f32; 4], Option<f32>)> {
        let fin = &self.fin;
        let ok = self.prog.fs.run(&mut self.m, &self.prog.fs_uniforms, self.tex, &mut |io, lanes| Self::fill(fin, frag, vary, io, lanes));
        if !ok { return None; }
        let out = &self.m.out;
        Some(self.result(|k| out[k as usize]))
    }
    fn shade_many(&mut self, frags: &[Frag], vary: &[f32], stride: usize, out: &mut [Option<([f32; 4], Option<f32>)>]) {
        let mut wm = self.wide.take().unwrap_or_else(|| self.prog.fs.wide_machine());
        let fs = &self.prog.fs;
        for (b, (fr, o)) in frags.chunks(exec::WIDE).zip(out.chunks_mut(exec::WIDE)).enumerate() {
            let base = b * exec::WIDE;
            for &(io, off, n) in &fs.inputs {
                match io {
                    Io::Location(_) => {
                        let Some(&(_, fnum, Some(at))) = self.fin.iter().find(|(i, _, _)| *i == io) else { continue };
                        for k in 0..n.min(fnum) {
                            let row = wm.reg(off + k);
                            for (l, v) in row.iter_mut().enumerate().take(fr.len()) {
                                *v = vary.get((base + l) * stride + (at + k) as usize).copied().unwrap_or(0.0).to_bits();
                            }
                        }
                    }
                    Io::Position => for k in 0..n.min(4) {
                        let row = wm.reg(off + k);
                        for (l, f) in fr.iter().enumerate() { row[l] = [f.x, f.y, f.z, f.inv_w][k as usize].to_bits(); }
                    },
                    Io::FrontFacing => { let row = wm.reg(off); for (l, f) in fr.iter().enumerate() { row[l] = f.front as u32; } }
                    Io::PointCoord => for k in 0..n.min(2) {
                        let row = wm.reg(off + k);
                        for (l, f) in fr.iter().enumerate() { row[l] = f.point[k as usize].to_bits(); }
                    },
                    _ => {}
                }
            }
            let live = fs.run_rows(&mut wm, fr.len(), &self.prog.fs_uniforms, self.tex);
            for (l, slot) in o.iter_mut().enumerate() {
                *slot = if (live >> l) & 1 != 0 { Some(self.result(|k| wm.out(l, k))) } else { None };
            }
        }
        self.wide = Some(wm);
    }
}
