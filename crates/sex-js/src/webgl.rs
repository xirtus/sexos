// SPDX-License-Identifier: GPL-3.0-only
//! The host side of WebGL: runs the command batches the prelude sends for each canvas
//! (fields split by U+001F, commands by U+001E; bulk data in one ArrayBuffer per batch,
//! referenced as offset and length) on a sex-gl context, and answers queries.
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sex_gl::Context;

/// One canvas's WebGL context.
pub struct GlCanvas {
    pub ctx: Context,
    pub alpha: bool,
    /// The frame last shown on the page (compare with `ctx.frame`).
    pub shown: u64,
    /// Picture uploads waiting for their pixels: (target, texture, address, flip y,
    /// premultiply).
    pub pictures: Vec<(u32, u32, String, bool, bool)>,
}

fn n<T: core::str::FromStr + Default>(f: &[&str], i: usize) -> T { f.get(i).and_then(|s| s.parse().ok()).unwrap_or_default() }
fn fl(f: &[&str], i: usize) -> f32 { f.get(i).and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0) }

fn blob<'a>(data: &'a [u8], f: &[&str], i: usize) -> Option<&'a [u8]> {
    let off: usize = f.get(i)?.parse().ok()?;
    let len: usize = f.get(i + 1)?.parse().ok()?;
    data.get(off..off.checked_add(len)?)
}

fn f32s(b: &[u8]) -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() }
fn i32s(b: &[u8]) -> Vec<i32> { b.chunks_exact(4).map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() }

impl GlCanvas {
    pub fn new(width: u32, height: u32, alpha: bool) -> Self { GlCanvas { ctx: Context::new(width, height), alpha, shown: u64::MAX, pictures: Vec::new() } }

    /// Runs one batch.
    pub fn run(&mut self, cmds: &str, data: &[u8], pictures: &[(String, alloc::rc::Rc<sex_image::Image>)]) {
        let c = &mut self.ctx;
        for cmd in cmds.split('\u{1e}') {
            let f: Vec<&str> = cmd.split('\u{1f}').collect();
            let a = &f[1..];
            match f[0] {
                "en" => c.enable(n(a, 0), n::<u32>(a, 1) != 0),
                "vp" => c.viewport(n(a, 0), n(a, 1), n(a, 2), n(a, 3)),
                "sc" => c.scissor(n(a, 0), n(a, 1), n(a, 2), n(a, 3)),
                "cc" => c.clear_color([fl(a, 0), fl(a, 1), fl(a, 2), fl(a, 3)]),
                "cd" => c.clear_depth(fl(a, 0)),
                "cm" => c.color_mask(n::<u32>(a, 0) != 0, n::<u32>(a, 1) != 0, n::<u32>(a, 2) != 0, n::<u32>(a, 3) != 0),
                "dm" => c.depth_mask(n::<u32>(a, 0) != 0),
                "df" => c.depth_func(n(a, 0)),
                "dr" => c.depth_range(fl(a, 0), fl(a, 1)),
                "cf" => c.cull_face(n(a, 0)),
                "ff" => c.front_face(n(a, 0)),
                "bf" => c.blend_func(n(a, 0), n(a, 1), n(a, 2), n(a, 3)),
                "be" => c.blend_equation(n(a, 0), n(a, 1)),
                "bc" => c.blend_color([fl(a, 0), fl(a, 1), fl(a, 2), fl(a, 3)]),
                "po" => c.polygon_offset(fl(a, 0), fl(a, 1)),
                "ps" => c.pixel_store(n(a, 0), n(a, 1)),
                "cl" => c.clear(n(a, 0)),
                "fb" => c.bind_framebuffer(n(a, 0)),
                "ft" => c.framebuffer_texture(n(a, 0), n(a, 1), n(a, 2)),
                "fr" => c.framebuffer_renderbuffer(n(a, 0), n(a, 1)),
                "rb" => c.bind_renderbuffer(n(a, 0)),
                "rs" => c.renderbuffer_storage(n(a, 0), n(a, 1)),
                "dfb" => c.delete_framebuffer(n(a, 0)),
                "drb" => c.delete_renderbuffer(n(a, 0)),
                "blt" => c.blit_framebuffer(n(a, 0), n(a, 1), [n(a, 2), n(a, 3), n(a, 4), n(a, 5)], [n(a, 6), n(a, 7), n(a, 8), n(a, 9)]),
                "rz" => c.resize(n(a, 0), n(a, 1)),
                "bb" => c.bind_buffer(n(a, 0), n(a, 1)),
                "bd" => { let d = blob(data, a, 1).unwrap_or(&[]); c.buffer_data(n(a, 0), d) }
                "bz" => { let size: usize = n(a, 1); c.buffer_data(n(a, 0), &alloc::vec![0u8; size.min(256 << 20)]) }
                "bs" => if let Some(d) = blob(data, a, 2) { c.buffer_sub_data(n(a, 0), n(a, 1), d) },
                "db" => c.delete_buffer(n(a, 0)),
                "ba" => c.bind_vertex_array(n(a, 0)),
                "dva" => c.delete_vertex_array(n(a, 0)),
                "ea" => c.enable_attrib(n(a, 0), n::<u32>(a, 1) != 0),
                "ap" => c.attrib_pointer(n(a, 0), n(a, 1), n(a, 2), n::<u32>(a, 3) != 0, n(a, 4), n(a, 5)),
                "ad" => c.attrib_divisor(n(a, 0), n(a, 1)),
                "va" => c.vertex_attrib(n(a, 0), [fl(a, 1), fl(a, 2), fl(a, 3), fl(a, 4)]),
                "at" => c.active_texture(n(a, 0)),
                "bt" => c.bind_texture(n(a, 0), n(a, 1)),
                "dt" => c.delete_texture(n(a, 0)),
                "tp" => c.tex_parameter(n(a, 0), n(a, 1), n(a, 2)),
                "ti" => {
                    let d = blob(data, a, 6);
                    c.tex_image(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5), d);
                }
                // ti3 target level w h depth format type [data]; ts3 target level x y z w h d format type data
                "ti3" => {
                    let d = blob(data, a, 7);
                    c.tex_image_3d(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5), n(a, 6), d);
                }
                "ts3" => if let Some(d) = blob(data, a, 10) { c.tex_sub_image_3d(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5), n(a, 6), n(a, 7), n(a, 8), n(a, 9), d) },
                "tP" => picture(&mut self.pictures, c, a, pictures),
                "ts" => if let Some(d) = blob(data, a, 8) { c.tex_sub_image(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5), n(a, 6), n(a, 7), d) },
                "cs" => c.create_shader(n(a, 0), n(a, 1)),
                "ss" => if let Some(d) = blob(data, a, 1) { c.shader_source(n(a, 0), core::str::from_utf8(d).unwrap_or("")) },
                "co" => c.compile_shader(n(a, 0)),
                "cp" => c.create_program(n(a, 0)),
                "as" => c.attach_shader(n(a, 0), n(a, 1)),
                "ds" => c.delete_shader(n(a, 0)),
                "dp" => c.delete_program(n(a, 0)),
                "bl" => c.bind_attrib_location(n(a, 0), n(a, 1), a.get(2).copied().unwrap_or("")),
                "lp" => c.link_program(n(a, 0)),
                "up" => c.use_program(n(a, 0)),
                "uf" => if let Some(d) = blob(data, a, 1) { c.uniform(n(a, 0), Some(&f32s(d)), None) },
                "ui" => if let Some(d) = blob(data, a, 1) { c.uniform(n(a, 0), None, Some(&i32s(d))) },
                "da" => c.draw_arrays(n(a, 0), n(a, 1), n(a, 2)),
                "de" => c.draw_elements(n(a, 0), n(a, 1), n(a, 2), n(a, 3)),
                "dai" => c.draw_arrays_instanced(n(a, 0), n(a, 1), n(a, 2), n(a, 3)),
                "dei" => c.draw_elements_instanced(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4)),
                "aip" => c.attrib_ipointer(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4)),
                // WebGPU: gp id vs-code(off len) vs-entry fs-code(off len) fs-entry;
                // gb/gx/gs program group binding ...
                "gp" => {
                    let text = |i| blob(data, a, i).and_then(|d| core::str::from_utf8(d).ok()).unwrap_or("");
                    let entry = |i: usize| a.get(i).copied().unwrap_or("");
                    c.gpu_program(n(a, 0), text(1), entry(3), text(4), entry(6));
                }
                "gb" => c.gpu_bind_buffer(n(a, 0), n(a, 1), n(a, 2), n(a, 3), n(a, 4), n(a, 5)),
                "gx" => c.gpu_bind_texture(n(a, 0), n(a, 1), n(a, 2), n(a, 3)),
                "gs" => c.gpu_bind_sampler(n(a, 0), n(a, 1), n(a, 2), [n(a, 3), n(a, 4), n(a, 5), n(a, 6)]),
                _ => {}
            }
        }
    }

    /// Answers a query (the batch before it has run).
    pub fn query(&mut self, what: &str, a: &[&[u8]]) -> Option<Vec<u8>> {
        let s = |i: usize| a.get(i).and_then(|b| core::str::from_utf8(b).ok()).unwrap_or("");
        let num = |i: usize| s(i).parse::<u32>().unwrap_or(0);
        let c = &mut self.ctx;
        let text = match what {
            "shader" => { let (ok, log) = c.shader_status(num(0)); format!("{}\u{1f}{}", ok as u8, log) }
            "program" => match c.programs.get(&num(0)) {
                Some(p) => {
                    let (u, at) = p.linked.as_ref().map_or((0, 0), |l| (l.uniforms.len(), l.attributes.len()));
                    format!("{}\u{1f}{}\u{1f}{}\u{1f}{}", p.linked.is_some() as u8, p.log, u, at)
                }
                None => String::from("0\u{1f}\u{1f}0\u{1f}0"),
            },
            "uniforms" => {
                let Some(p) = c.program(num(0)) else { return Some(Vec::new()) };
                let mut out = String::new();
                for (i, u) in p.uniforms.iter().enumerate() {
                    if i > 0 { out.push('\u{1e}'); }
                    let name = if u.size > 1 { format!("{}[0]", u.name) } else { u.name.clone() };
                    out.push_str(&format!("{name}\u{1f}{}\u{1f}{}", u.gl_type, u.size));
                }
                out
            }
            "attribs" => {
                let Some(p) = c.program(num(0)) else { return Some(Vec::new()) };
                let mut out = String::new();
                for (i, (name, loc, ty, size)) in p.attributes.iter().enumerate() {
                    if i > 0 { out.push('\u{1e}'); }
                    out.push_str(&format!("{name}\u{1f}{ty}\u{1f}{size}\u{1f}{loc}"));
                }
                out
            }
            "uloc" => match c.uniform_location(num(0), s(1)) { Some(l) => format!("{l}"), None => String::new() },
            "aloc" => match c.program(num(0)).and_then(|p| p.attributes.iter().find(|x| x.0 == s(1))) { Some(x) => format!("{}", x.1), None => String::from("-1") },
            "source" => { c.shader_source(num(0), s(1)); return Some(Vec::new()) }
            "error" => format!("{}", c.get_error()),
            "read" => return Some(c.read_pixels(s(0).parse().unwrap_or(0), s(1).parse().unwrap_or(0), num(2).min(4096), num(3).min(4096))),
            "enabled" => format!("{}", c.is_enabled(num(0)) as u8),
            _ => return None,
        };
        Some(text.into_bytes())
    }
}

/// tP target texture src: a page picture is uploaded now. A canvas's picture is drawn by
/// the browser after the scripts, so it arrives after the batch, with the unpack state of
/// this moment.
fn picture(pending: &mut Vec<(u32, u32, String, bool, bool)>, c: &mut Context, a: &[&str], pictures: &[(String, alloc::rc::Rc<sex_image::Image>)]) {
    let target: u32 = n(a, 0);
    let tex: u32 = n(a, 1);
    let src = a.get(2).copied().unwrap_or("");
    match pictures.iter().find(|(s, _)| s == src) {
        Some((_, img)) => {
            let (flip, premul) = (c.flip_y, c.premultiply);
            c.upload_picture(target, tex, img.width as u32, img.height as u32, &img.pixels, flip, premul);
        }
        None => pending.push((target, tex, src.into(), c.flip_y, c.premultiply)),
    }
}

/// All WebGL canvases of a page.
#[derive(Default)]
pub struct GlCanvases {
    pub by_node: BTreeMap<usize, GlCanvas>,
}
