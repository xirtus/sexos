// SPDX-License-Identifier: GPL-3.0-only
//! Times full-screen draws of a few fragment shaders: `cargo run --release -p sex-gl --example shaders`.
//! Prints ms per frame and a hash of the last frame, so a faster engine can show it draws the same pixels.
use sex_gl::enums::*;
use sex_gl::raster::gl;
use sex_gl::Context;

fn bytes(v: &[f32]) -> Vec<u8> { v.iter().flat_map(|x| x.to_le_bytes()).collect() }

const VS: &str = "attribute vec2 a; varying vec2 v; void main(){ v = a; gl_Position = vec4(a, 0.0, 1.0); }";
const SHADERS: &[(&str, &str)] = &[
    ("flat", "precision mediump float; varying vec2 v; void main(){ gl_FragColor = vec4(v * 0.5 + 0.5, 0.5, 1.0); }"),
    ("trig", "precision mediump float; varying vec2 v; uniform float t;
void main(){ vec2 q = v * 3.0; float s = 0.0; for (int i = 0; i < 4; i++) { s += sin(q.x * float(i + 1) + t) * cos(q.y * float(i + 2) - t); }
gl_FragColor = vec4(0.5 + 0.5 * sin(s), 0.5 + 0.5 * cos(s * 1.3), 0.5 + 0.25 * s, 1.0); }"),
    ("lit", "precision mediump float; varying vec2 v; uniform float t;
void main(){ vec3 n = normalize(vec3(v, 1.0)); vec3 l = normalize(vec3(0.3, 0.5, 1.0)); vec3 h = normalize(l + vec3(0.0, 0.0, 1.0));
float d = max(dot(n, l), 0.0); float s = pow(max(dot(n, h), 0.0), 32.0);
gl_FragColor = vec4(vec3(0.2, 0.4, 0.8) * (0.2 + 0.8 * d) + vec3(s), 1.0); }"),
];

fn main() {
    let (w, h) = (640, 400);
    for (name, fs) in SHADERS {
        let mut ctx = Context::new(w, h);
        ctx.create_shader(1, VERTEX_SHADER); ctx.shader_source(1, VS); ctx.compile_shader(1);
        ctx.create_shader(2, FRAGMENT_SHADER); ctx.shader_source(2, fs); ctx.compile_shader(2);
        assert!(ctx.shader_status(2).0, "{}", ctx.shader_status(2).1);
        ctx.create_program(3); ctx.attach_shader(3, 1); ctx.attach_shader(3, 2); ctx.link_program(3); ctx.use_program(3);
        ctx.bind_buffer(ARRAY_BUFFER, 10);
        ctx.buffer_data(ARRAY_BUFFER, &bytes(&[-1., -1., 1., -1., 1., 1., -1., -1., 1., 1., -1., 1.]));
        let a = ctx.program(3).unwrap().attributes.iter().find(|x| x.0 == "a").unwrap().1;
        ctx.enable_attrib(a, true);
        ctx.attrib_pointer(a, 2, FLOAT, false, 0, 0);
        if std::env::var("STEPS").is_ok() { for (i, st) in ctx.program(3).unwrap().fs.describe().iter().enumerate() { println!("{i:3} {st}"); } }
        let t = ctx.uniform_location(3, "t");
        if std::env::var("ONLY").is_ok_and(|o| o != *name) { continue; }
        let frames = std::env::var("FRAMES").ok().and_then(|f| f.parse().ok()).unwrap_or(20);
        let start = std::time::Instant::now();
        for f in 0..frames {
            if let Some(t) = t { ctx.uniform(t, Some(&[f as f32 * 0.1]), None); }
            ctx.draw_arrays(gl::TRIANGLES, 0, 6);
        }
        let ns = start.elapsed().as_nanos() as f64 / (frames as f64 * (w * h) as f64);
        let hash = ctx.color.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, v| (h ^ *v as u64).wrapping_mul(0x1_0000_0001_b3));
        println!("{name:6} {:7.2} ms/frame {:6.1} ns/pixel hash {hash:016x}", ns * (w * h) as f64 / 1e6, ns);
    }
}
