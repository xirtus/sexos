// SPDX-License-Identifier: GPL-3.0-only
use sex_gl::enums::*;
use sex_gl::raster::gl;
use sex_gl::Context;

fn bytes(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

fn program(ctx: &mut Context, vs: &str, fs: &str) -> u32 {
    ctx.create_shader(1, VERTEX_SHADER);
    ctx.shader_source(1, vs);
    ctx.compile_shader(1);
    let (ok, log) = ctx.shader_status(1);
    assert!(ok, "vs: {log}");
    ctx.create_shader(2, FRAGMENT_SHADER);
    ctx.shader_source(2, fs);
    ctx.compile_shader(2);
    let (ok, log) = ctx.shader_status(2);
    assert!(ok, "fs: {log}");
    ctx.create_program(3);
    ctx.attach_shader(3, 1);
    ctx.attach_shader(3, 2);
    ctx.link_program(3);
    assert!(ctx.program(3).is_some(), "link: {}", ctx.programs[&3].log);
    ctx.use_program(3);
    3
}

fn px(ctx: &Context, x: u32, y: u32) -> [u8; 4] { ctx.color[(y * ctx.width + x) as usize].to_le_bytes() }

/// Writes the frame as a PPM next to the test binary's target dir, for looking at.
fn save(ctx: &Context, name: &str) {
    let img = ctx.present(false);
    let mut out = format!("P6\n{} {}\n255\n", ctx.width, ctx.height).into_bytes();
    for p in img { out.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8]); }
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    std::fs::write(dir.join(name), out).unwrap();
}

#[test]
fn triangle_fills_with_interpolated_color() {
    let mut ctx = Context::new(64, 64);
    program(&mut ctx, "attribute vec2 p; attribute vec3 c; varying vec3 v; void main(){ v = c; gl_Position = vec4(p, 0.0, 1.0); }",
        "precision mediump float; varying vec3 v; void main(){ gl_FragColor = vec4(v, 1.0); }");
    ctx.bind_buffer(ARRAY_BUFFER, 10);
    ctx.buffer_data(ARRAY_BUFFER, &bytes(&[-1., -1., 1., 0., 0., 1., -1., 0., 1., 0., -1., 1., 0., 0., 1.]));
    let p = ctx.program(3).unwrap().attributes.iter().find(|a| a.0 == "p").unwrap().1;
    let c = ctx.program(3).unwrap().attributes.iter().find(|a| a.0 == "c").unwrap().1;
    ctx.enable_attrib(p, true);
    ctx.attrib_pointer(p, 2, FLOAT, false, 20, 0);
    ctx.enable_attrib(c, true);
    ctx.attrib_pointer(c, 3, FLOAT, false, 20, 8);
    ctx.clear_color([0.0, 0.0, 0.0, 1.0]);
    ctx.clear(COLOR_BUFFER_BIT);
    ctx.draw_arrays(gl::TRIANGLES, 0, 3);
    assert_eq!(ctx.get_error(), NO_ERROR);
    save(&ctx, "triangle.ppm");
    // Bottom-left corner is red, bottom-right green, top-left blue.
    let bl = px(&ctx, 1, 1);
    assert!(bl[0] > 240 && bl[1] < 10 && bl[2] < 10, "{bl:?}");
    let br = px(&ctx, 61, 1);
    assert!(br[1] > 200 && br[0] < 30, "{br:?}");
    let tl = px(&ctx, 1, 61);
    assert!(tl[2] > 200 && tl[0] < 30, "{tl:?}");
    // Outside the triangle stays clear.
    assert_eq!(px(&ctx, 62, 62), [0, 0, 0, 255]);
}

const CUBE_VS: &str = r#"
attribute vec3 aPos;
attribute vec3 aNormal;
attribute vec2 aUv;
uniform mat4 uProj;
uniform mat4 uModelView;
uniform mat3 uNormalMat;
varying vec3 vNormal;
varying vec2 vUv;
void main() {
  vNormal = uNormalMat * aNormal;
  vUv = aUv;
  gl_Position = uProj * uModelView * vec4(aPos, 1.0);
}"#;

const CUBE_FS: &str = r#"
precision mediump float;
uniform sampler2D uTex;
uniform vec3 uLight;
varying vec3 vNormal;
varying vec2 vUv;
void main() {
  float d = max(dot(normalize(vNormal), normalize(uLight)), 0.0);
  vec3 base = texture2D(uTex, vUv).rgb;
  gl_FragColor = vec4(base * (0.25 + 0.75 * d), 1.0);
}"#;

fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0f32; 16];
    for c in 0..4 { for r in 0..4 { o[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum(); } }
    o
}

fn cube() -> (Vec<f32>, Vec<u16>) {
    // 6 faces x 4 verts: pos(3) normal(3) uv(2)
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        ([0., 0., 1.], [1., 0., 0.], [0., 1., 0.]),
        ([0., 0., -1.], [-1., 0., 0.], [0., 1., 0.]),
        ([1., 0., 0.], [0., 0., -1.], [0., 1., 0.]),
        ([-1., 0., 0.], [0., 0., 1.], [0., 1., 0.]),
        ([0., 1., 0.], [1., 0., 0.], [0., 0., -1.]),
        ([0., -1., 0.], [1., 0., 0.], [0., 0., 1.]),
    ];
    let mut v = Vec::new();
    let mut idx = Vec::new();
    for (f, (n, u, w)) in faces.iter().enumerate() {
        for (s, t) in [(-1f32, -1f32), (1., -1.), (1., 1.), (-1., 1.)] {
            for k in 0..3 { v.push(n[k] + u[k] * s + w[k] * t); }
            v.extend_from_slice(n);
            v.push((s + 1.) / 2.);
            v.push((t + 1.) / 2.);
        }
        let b = (f * 4) as u16;
        idx.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    (v, idx)
}

#[test]
fn lit_textured_cube_with_depth() {
    let mut ctx = Context::new(160, 120);
    program(&mut ctx, CUBE_VS, CUBE_FS);
    let (v, idx) = cube();
    ctx.bind_buffer(ARRAY_BUFFER, 10);
    ctx.buffer_data(ARRAY_BUFFER, &bytes(&v));
    ctx.bind_buffer(ELEMENT_ARRAY_BUFFER, 11);
    ctx.buffer_data(ELEMENT_ARRAY_BUFFER, &idx.iter().flat_map(|i| i.to_le_bytes()).collect::<Vec<_>>());
    let attrs = ctx.program(3).unwrap().attributes.clone();
    let loc = |n: &str| attrs.iter().find(|a| a.0 == n).unwrap().1;
    for (name, size, off) in [("aPos", 3, 0), ("aNormal", 3, 12), ("aUv", 2, 24)] {
        ctx.enable_attrib(loc(name), true);
        ctx.attrib_pointer(loc(name), size, FLOAT, false, 32, off);
    }
    // A 2x2 checker texture: orange and white.
    ctx.active_texture(TEXTURE0);
    ctx.bind_texture(TEXTURE_2D, 20);
    ctx.tex_image(TEXTURE_2D, 0, 2, 2, RGBA, UNSIGNED_BYTE, Some(&[255, 128, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 128, 0, 255]));
    ctx.tex_parameter(TEXTURE_2D, TEXTURE_MAG_FILTER, NEAREST);
    ctx.tex_parameter(TEXTURE_2D, TEXTURE_MIN_FILTER, NEAREST);
    let (f, aspect, near, far) = (1.0 / (0.4f32).tan(), 160.0 / 120.0, 0.1f32, 100.0f32);
    let proj = [f / aspect, 0., 0., 0., 0., f, 0., 0., 0., 0., (far + near) / (near - far), -1., 0., 0., 2. * far * near / (near - far), 0.];
    let (a, b) = (0.6f32, 0.5f32);
    let ry = [a.cos(), 0., -a.sin(), 0., 0., 1., 0., 0., a.sin(), 0., a.cos(), 0., 0., 0., 0., 1.];
    let rx = [1., 0., 0., 0., 0., b.cos(), b.sin(), 0., 0., -b.sin(), b.cos(), 0., 0., 0., 0., 1.];
    let tr = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., -5., 1.];
    let mv = mat_mul(&tr, &mat_mul(&rx, &ry));
    let nm = [mv[0], mv[1], mv[2], mv[4], mv[5], mv[6], mv[8], mv[9], mv[10]];
    let u = |ctx: &mut Context, n: &str| ctx.uniform_location(3, n).unwrap_or_else(|| panic!("no uniform {n}"));
    let (lp, lmv, ln, ll, lt) = (u(&mut ctx, "uProj"), u(&mut ctx, "uModelView"), u(&mut ctx, "uNormalMat"), u(&mut ctx, "uLight"), u(&mut ctx, "uTex"));
    ctx.uniform(lp, Some(&proj), None);
    ctx.uniform(lmv, Some(&mv), None);
    ctx.uniform(ln, Some(&nm), None);
    ctx.uniform(ll, Some(&[0.3, 0.6, 1.0]), None);
    ctx.uniform(lt, None, Some(&[0]));
    ctx.enable(DEPTH_TEST, true);
    ctx.enable(CULL_FACE, true);
    ctx.clear_color([0.1, 0.1, 0.2, 1.0]);
    ctx.clear(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT);
    ctx.draw_elements(gl::TRIANGLES, 36, UNSIGNED_SHORT, 0);
    assert_eq!(ctx.get_error(), NO_ERROR);
    save(&ctx, "cube.ppm");
    // The centre is the cube, lit; corners are the clear colour.
    let c = px(&ctx, 80, 60);
    assert!(c[0] > 60, "centre {c:?}");
    assert_eq!(px(&ctx, 2, 2), [26, 26, 51, 255]);
    // Both checker colours appear.
    let (mut orange, mut white) = (0, 0);
    for p in &ctx.color {
        let [r, g, b, _] = p.to_le_bytes().map(|c| c as u32);
        if r > 100 && b < r / 3 && g < r * 3 / 4 { orange += 1; }
        if r > 100 && g > r * 9 / 10 && b > r * 9 / 10 { white += 1; }
    }
    assert!(orange > 300 && white > 300, "orange {orange} white {white}");
}

#[test]
fn es3_shaders_blending_and_scissor() {
    let mut ctx = Context::new(32, 32);
    program(&mut ctx, "#version 300 es\nin vec2 p;\nvoid main(){ gl_Position = vec4(p, 0.5, 1.0); }",
        "#version 300 es\nprecision highp float;\nuniform vec4 col;\nout vec4 o;\nvoid main(){ o = col; }");
    ctx.bind_buffer(ARRAY_BUFFER, 10);
    ctx.buffer_data(ARRAY_BUFFER, &bytes(&[-1., -1., 3., -1., -1., 3.]));
    ctx.enable_attrib(0, true);
    ctx.attrib_pointer(0, 2, FLOAT, false, 0, 0);
    ctx.clear_color([0.0, 0.0, 1.0, 1.0]);
    ctx.clear(COLOR_BUFFER_BIT);
    let l = ctx.uniform_location(3, "col").unwrap();
    ctx.uniform(l, Some(&[1.0, 0.0, 0.0, 0.5]), None);
    ctx.enable(BLEND, true);
    ctx.blend_func(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE, gl::ONE);
    ctx.enable(SCISSOR_TEST, true);
    ctx.scissor(0, 0, 16, 32);
    ctx.draw_arrays(gl::TRIANGLES, 0, 3);
    let left = px(&ctx, 4, 4);
    assert!((left[0] as i32 - 128).abs() <= 1 && (left[2] as i32 - 128).abs() <= 1, "{left:?}");
    assert_eq!(px(&ctx, 20, 4), [0, 0, 255, 255]);
}

#[test]
fn shader_depth_and_declared_attribute_locations() {
    let mut ctx = Context::new(8, 8);
    // `p` declares location 3; `q` takes a free one. The depth the shader writes (0.25)
    // passes LESS against 0.5, though the triangle's own depth (0.75) would not.
    program(&mut ctx, "#version 300 es\nlayout(location = 3) in vec2 p;\nin float q;\nvoid main(){ gl_Position = vec4(p, 0.5 * q, 1.0); }",
        "#version 300 es\nprecision highp float;\nout vec4 o;\nvoid main(){ o = vec4(0.0, 1.0, 0.0, 1.0); gl_FragDepth = 0.25; }");
    let attrs = &ctx.program(3).unwrap().attributes;
    assert_eq!(attrs.iter().find(|a| a.0 == "p").unwrap().1, 3);
    assert_eq!(attrs.iter().find(|a| a.0 == "q").unwrap().1, 0);
    ctx.bind_buffer(ARRAY_BUFFER, 10);
    ctx.buffer_data(ARRAY_BUFFER, &bytes(&[-1., -1., 3., -1., -1., 3.]));
    ctx.enable_attrib(3, true);
    ctx.attrib_pointer(3, 2, FLOAT, false, 0, 0);
    ctx.vertex_attrib(0, [1.0, 0.0, 0.0, 1.0]);
    ctx.clear_color([0.0, 0.0, 0.0, 1.0]);
    ctx.clear_depth(0.5);
    ctx.clear(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT);
    ctx.enable(DEPTH_TEST, true);
    ctx.draw_arrays(gl::TRIANGLES, 0, 3);
    assert_eq!(px(&ctx, 4, 4), [0, 255, 0, 255]);
}

/// Neighbouring pixels take different branches, loop counts, calls and discards; they
/// are shaded together, so each must still get exactly its own result.
#[test]
fn divergent_branches_loops_and_discard_per_pixel() {
    let mut ctx = Context::new(16, 16);
    program(&mut ctx, "attribute vec2 p; void main(){ gl_Position = vec4(p, 0.0, 1.0); }", r#"
precision highp float;
float f(float x) { if (x > 0.5) { return x * 2.0; } return x * 0.5; }
void main() {
  float x = floor(gl_FragCoord.x);
  float y = floor(gl_FragCoord.y);
  if (mod(x, 5.0) == 0.0 && y > 7.0) discard;
  float s = 0.0;
  for (int i = 0; i < 16; i++) {
    if (float(i) >= x) break;
    if (mod(float(i), 2.0) == 0.0) continue;
    s += 1.0;
  }
  float c = mod(x + y, 3.0) == 0.0 ? f(0.75) : f(0.25);
  gl_FragColor = vec4(s / 16.0, c / 2.0, y / 16.0, 1.0);
}"#);
    ctx.bind_buffer(ARRAY_BUFFER, 10);
    ctx.buffer_data(ARRAY_BUFFER, &bytes(&[-1., -1., 1., -1., 1., 1., -1., -1., 1., 1., -1., 1.]));
    let p = ctx.program(3).unwrap().attributes.iter().find(|a| a.0 == "p").unwrap().1;
    ctx.enable_attrib(p, true);
    ctx.attrib_pointer(p, 2, FLOAT, false, 8, 0);
    ctx.clear_color([0.0, 0.0, 0.0, 0.0]);
    ctx.clear(COLOR_BUFFER_BIT);
    ctx.draw_arrays(gl::TRIANGLES, 0, 6);
    assert_eq!(ctx.get_error(), NO_ERROR);
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for y in 0..16u32 {
        for x in 0..16u32 {
            let want = if x % 5 == 0 && y > 7 { [0, 0, 0, 0] } else {
                let s = (x / 2) as f32;
                let c = if (x + y) % 3 == 0 { 1.5 } else { 0.125 };
                [b(s / 16.0), b(c / 2.0), b(y as f32 / 16.0), 255]
            };
            assert_eq!(px(&ctx, x, y), want, "pixel {x},{y}");
        }
    }
}

/// A WebGPU pipeline: WGSL with a uniform struct in WGSL byte layout (a vec3 then a
/// float in its padding, a mat2x2 with 8-byte columns), a texture and a sampler, and
/// depth from 0 (near) to 1.
#[test]
fn wgsl_pipeline_with_uniform_layout_texture_and_sampler() {
    let mut ctx = Context::new(16, 16);
    ctx.gpu_program(5, r#"
struct U { tint: vec3f, scale: f32, m: mat2x2f, z: f32 };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var t: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;
struct V { @builtin(position) pos: vec4f, @location(0) uv: vec2f };
@vertex fn vs(@location(0) p: vec2f) -> V {
  var o: V;
  o.pos = vec4f(u.m * p * u.scale, u.z, 1.0);
  o.uv = p * 0.5 + vec2f(0.5);
  return o;
}
@fragment fn fs(v: V) -> @location(0) vec4f {
  return vec4f(textureSample(t, s, v.uv).rgb * u.tint, 1.0);
}"#, "vs", "fs");
    let log = ctx.programs[&5].log.clone();
    assert!(ctx.program(5).is_some(), "{log}");
    // tint (0.5, 1, 1), scale 1 at byte 12, m = identity at byte 16 (columns 8 bytes
    // apart), z 0.5 at byte 32; struct size 48.
    let mut ub = bytes(&[0.5, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.5]);
    ub.resize(48, 0);
    ctx.bind_buffer(ARRAY_BUFFER, 20);
    ctx.buffer_data(ARRAY_BUFFER, &ub);
    ctx.use_program(5);
    ctx.gpu_bind_buffer(5, 0, 0, 20, 0, 48);
    // A 1x1 white texture on unit 3, nearest and clamped by the sampler.
    ctx.active_texture(TEXTURE0 + 3);
    ctx.bind_texture(TEXTURE_2D, 30);
    ctx.tex_image(TEXTURE_2D, 0, 1, 1, RGBA, UNSIGNED_BYTE, Some(&[255, 255, 255, 255]));
    ctx.gpu_bind_texture(5, 0, 1, 3);
    ctx.gpu_bind_sampler(5, 0, 2, [NEAREST, NEAREST, CLAMP_TO_EDGE, CLAMP_TO_EDGE]);
    assert_eq!(ctx.textures[&30].min_filter, NEAREST);
    ctx.bind_buffer(ARRAY_BUFFER, 10);
    ctx.buffer_data(ARRAY_BUFFER, &bytes(&[-1., -1., 1., -1., 1., 1., -1., -1., 1., 1., -1., 1.]));
    ctx.enable_attrib(0, true);
    ctx.attrib_pointer(0, 2, FLOAT, false, 8, 0);
    ctx.enable(DEPTH_TEST, true);
    ctx.clear_color([0.0, 0.0, 0.0, 1.0]);
    ctx.clear(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT);
    ctx.draw_arrays(gl::TRIANGLES, 0, 6);
    assert_eq!(ctx.get_error(), NO_ERROR);
    assert_eq!(px(&ctx, 8, 8), [128, 255, 255, 255]);
    // WebGPU depth: z 0.5 lands at 0.5 (GL would give 0.75).
    assert!((ctx.depth[8 * 16 + 8] - 0.5).abs() < 1e-6, "{}", ctx.depth[8 * 16 + 8]);
}
