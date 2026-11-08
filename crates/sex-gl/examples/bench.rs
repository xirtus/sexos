//! Times a lit, textured, depth-tested cube filling most of a 410x274 frame:
//! `cargo run --release -p sex-gl --example bench [frames]`. Prints ms per frame and a
//! hash of the last frame, so a faster renderer can show it draws the same pixels.
use sex_gl::enums::*;
use sex_gl::raster::gl;
use sex_gl::Context;

fn bytes(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

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


fn main() {
    let frames: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(10);
    let (w, h) = (410u32, 274u32);
    let mut ctx = Context::new(w, h);
    for (id, stage, src) in [(1, VERTEX_SHADER, CUBE_VS), (2, FRAGMENT_SHADER, CUBE_FS)] {
        ctx.create_shader(id, stage);
        ctx.shader_source(id, src);
        ctx.compile_shader(id);
    }
    ctx.create_program(3);
    ctx.attach_shader(3, 1);
    ctx.attach_shader(3, 2);
    ctx.link_program(3);
    ctx.use_program(3);
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
    let tex: Vec<u8> = (0..64 * 64).flat_map(|i| { let (x, y) = (i % 64, i / 64); if (x / 8 + y / 8) % 2 == 0 { [255, 128, 0, 255] } else { [240, 240, 255, 255] } }).collect();
    ctx.active_texture(TEXTURE0);
    ctx.bind_texture(TEXTURE_2D, 20);
    ctx.tex_image(TEXTURE_2D, 0, 64, 64, RGBA, UNSIGNED_BYTE, Some(&tex));
    ctx.tex_parameter(TEXTURE_2D, TEXTURE_MIN_FILTER, LINEAR);
    let (f, aspect, near, far) = (1.0 / (0.4f32).tan(), w as f32 / h as f32, 0.1f32, 100.0f32);
    let proj = [f / aspect, 0., 0., 0., 0., f, 0., 0., 0., 0., (far + near) / (near - far), -1., 0., 0., 2. * far * near / (near - far), 0.];
    let u = |ctx: &mut Context, n: &str| ctx.uniform_location(3, n).unwrap();
    let (lp, lmv, ln, ll, lt) = (u(&mut ctx, "uProj"), u(&mut ctx, "uModelView"), u(&mut ctx, "uNormalMat"), u(&mut ctx, "uLight"), u(&mut ctx, "uTex"));
    ctx.uniform(lp, Some(&proj), None);
    ctx.uniform(ll, Some(&[0.3, 0.6, 1.0]), None);
    ctx.uniform(lt, None, Some(&[0]));
    ctx.enable(DEPTH_TEST, true);
    ctx.enable(CULL_FACE, true);
    if std::env::var("SX_STEPS").is_ok() {
        for l in ctx.program(3).unwrap().fs.describe() { println!("{l}"); }
    }
    let start = std::time::Instant::now();
    for i in 0..frames {
        let (a, b) = (0.6f32 + i as f32 * 0.05, 0.5f32);
        let ry = [a.cos(), 0., -a.sin(), 0., 0., 1., 0., 0., a.sin(), 0., a.cos(), 0., 0., 0., 0., 1.];
        let rx = [1., 0., 0., 0., 0., b.cos(), b.sin(), 0., 0., -b.sin(), b.cos(), 0., 0., 0., 0., 1.];
        let tr = [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., -3.2, 1.];
        let mv = mat_mul(&tr, &mat_mul(&rx, &ry));
        let nm = [mv[0], mv[1], mv[2], mv[4], mv[5], mv[6], mv[8], mv[9], mv[10]];
        ctx.uniform(lmv, Some(&mv), None);
        ctx.uniform(ln, Some(&nm), None);
        ctx.clear_color([0.1, 0.1, 0.2, 1.0]);
        ctx.clear(COLOR_BUFFER_BIT | DEPTH_BUFFER_BIT);
        ctx.draw_elements(gl::TRIANGLES, 36, UNSIGNED_SHORT, 0);
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    let covered = ctx.color.iter().filter(|&&p| p != ctx.color[0]).count();
    let hash = ctx.color.iter().fold(0xcbf29ce484222325u64, |h, &p| (h ^ p as u64).wrapping_mul(0x100000001b3));
    println!("{ms:.2} ms/frame, {covered} shaded pixels, hash {hash:016x}");
}
