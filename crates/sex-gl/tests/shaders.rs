// SPDX-License-Identifier: GPL-3.0-only
use sex_gl::exec::{Io, Textures};
use sex_gl::glsl::Stage;
use sex_gl::program::{Program, Shader};
use std::collections::BTreeMap;

struct Tex;
impl Textures for Tex {
    fn sample(&self, _g: u32, c: [f32; 4], _: sex_gl::exec::Lod, _: Option<f32>) -> [f32; 4] { [c[0], c[1], 0.5, 1.0] }
}

fn f(v: &[f32]) -> Vec<u32> { v.iter().map(|x| x.to_bits()).collect() }
fn back(v: &[u32]) -> Vec<f32> { v.iter().map(|x| f32::from_bits(*x)).collect() }

const VS1: &str = "
attribute vec3 position;
attribute vec2 uv;
uniform mat4 mvp;
uniform float scale;
varying vec2 vUv;
void main() { vUv = uv * scale; gl_Position = mvp * vec4(position, 1.0); }";

const FS1: &str = "
precision mediump float;
uniform sampler2D map;
uniform vec3 tint;
varying vec2 vUv;
vec3 shade(vec3 c) { return c * tint; }
void main() {
  vec4 t = texture2D(map, vUv);
  if (t.r < 0.05) discard;
  float acc = 0.0;
  for (int i = 0; i < 4; i++) { acc += 0.25; }
  gl_FragColor = vec4(shade(t.rgb) * acc, mod(7.0, 4.0) / 4.0);
}";

#[test]
fn es1_program_links_and_runs() {
    let vs = Shader::compile(Stage::Vertex, VS1);
    let fs = Shader::compile(Stage::Fragment, FS1);
    assert!(vs.ok, "{}", vs.log);
    assert!(fs.ok, "{}", fs.log);
    let mut p = Program::link(&vs, &fs, &BTreeMap::new()).expect("link");
    let names: Vec<&str> = p.uniforms.iter().map(|u| u.name.as_str()).collect();
    for n in ["mvp", "scale", "tint", "map"] { assert!(names.contains(&n), "{names:?}"); }
    assert_eq!(p.attributes.len(), 2);

    // mvp = translate(1, 2, 3), scale = 2
    let (m, _) = p.locate("mvp").unwrap();
    p.set(m, 0, &f(&[1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 1., 2., 3., 1.]));
    let (s, _) = p.locate("scale").unwrap();
    p.set(s, 0, &f(&[2.0]));
    let (t, _) = p.locate("tint").unwrap();
    p.set(t, 0, &f(&[1.0, 0.5, 0.25]));

    let pos_loc = p.attributes.iter().find(|a| a.0 == "position").unwrap().1;
    let uv_loc = p.attributes.iter().find(|a| a.0 == "uv").unwrap().1;
    let mut m = p.vs.machine();
    let ok = p.vs.run(&mut m, &p.vs_uniforms, &Tex, &mut |io, lanes| match io {
        Io::Location(l) if l == pos_loc => lanes.copy_from_slice(&f(&[1.0, 1.0, 1.0])),
        Io::Location(l) if l == uv_loc => lanes.copy_from_slice(&f(&[0.25, 0.125])),
        _ => {}
    });
    assert!(ok);
    let out = back(&m.out);
    let pos = &out[p.position as usize..p.position as usize + 4];
    assert_eq!(pos, &[2.0, 3.0, 4.0, 1.0]);
    let (vo, fo, n) = p.links[0];
    assert_eq!(n, 2);
    assert_eq!(&out[vo as usize..vo as usize + 2], &[0.5, 0.25]);

    // Fragment: feed vUv straight through.
    let mut fm = p.fs.machine();
    let ok = p.fs.run(&mut fm, &p.fs_uniforms, &Tex, &mut |io, lanes| {
        if let Io::Location(_) = io { lanes.copy_from_slice(&f(&[0.5, 0.25])) }
    });
    let _ = fo;
    assert!(ok);
    let (co, cl) = p.color.unwrap();
    let c = back(&fm.out[co as usize..(co + cl) as usize]);
    assert_eq!(c, vec![0.5, 0.125, 0.125, 0.75]);

    // Discard.
    let ok = p.fs.run(&mut fm, &p.fs_uniforms, &Tex, &mut |io, lanes| {
        if let Io::Location(_) = io { lanes.copy_from_slice(&f(&[0.0, 0.25])) }
    });
    assert!(!ok);
}

// The forms three.js's shaders use: ES 3.00 with precision on declarations, `const in`
// parameters, boolean uniforms, structs before the uniforms that use them, and `modf`.
const VS3: &str = "#version 300 es
precision highp float;
in highp vec3 position;
uniform bool flip;
uniform mat4 m;
out vec3 vPos;
float twice(const in float x) { return 2.0 * x; }
void main() { vPos = position; gl_Position = m * vec4(flip ? -position : position, twice(0.5)); }";

const FS3: &str = "#version 300 es
precision highp float;
layout(location = 0) out highp vec4 outColor;
struct Light { vec3 color; float power; };
uniform Light light;
uniform bvec2 mask;
in vec3 vPos;
void main() {
  vec3 whole;
  vec3 part = modf(vPos, whole);
  outColor = vec4(mask.x ? light.color * light.power : part, whole.x);
}";

#[test]
fn es3_forms_from_libraries_compile_and_run() {
    let vs = Shader::compile(Stage::Vertex, VS3);
    let fs = Shader::compile(Stage::Fragment, FS3);
    assert!(vs.ok, "{}", vs.log);
    assert!(fs.ok, "{}", fs.log);
    let mut p = Program::link(&vs, &fs, &BTreeMap::new()).expect("link");
    let ty = |n: &str| p.uniforms.iter().find(|u| u.name == n).map(|u| u.gl_type);
    assert_eq!(ty("flip"), Some(sex_gl::program::types::BOOL));
    assert_eq!(ty("mask"), Some(sex_gl::program::types::BOOL_VEC2));
    assert_eq!(ty("light.power"), Some(sex_gl::program::types::FLOAT));
    let (m, _) = p.locate("m").unwrap();
    p.set(m, 0, &f(&[1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.]));
    let (fl, _) = p.locate("flip").unwrap();
    p.set(fl, 0, &[1]);
    let mut vm = p.vs.machine();
    assert!(p.vs.run(&mut vm, &p.vs_uniforms, &Tex, &mut |io, lanes| if let Io::Location(_) = io { lanes.copy_from_slice(&f(&[1.0, 2.0, 3.0])) }));
    let out = back(&vm.out);
    assert_eq!(&out[p.position as usize..p.position as usize + 4], &[-1.0, -2.0, -3.0, 1.0]);

    let mut fm = p.fs.machine();
    let run = |p: &Program, fm: &mut sex_gl::exec::Machine| {
        assert!(p.fs.run(fm, &p.fs_uniforms, &Tex, &mut |io, lanes| if let Io::Location(_) = io { lanes.copy_from_slice(&f(&[2.25, 1.5, 0.75])) }));
        let (co, cl) = p.color.unwrap();
        back(&fm.out[co as usize..(co + cl) as usize])
    };
    assert_eq!(run(&p, &mut fm), vec![0.25, 0.5, 0.75, 2.0]);
    let (c, _) = p.locate("light.color").unwrap();
    p.set(c, 0, &f(&[0.5, 0.25, 1.0]));
    let (w, _) = p.locate("light.power").unwrap();
    p.set(w, 0, &f(&[2.0]));
    let (mk, _) = p.locate("mask").unwrap();
    p.set(mk, 0, &[1, 0]);
    assert_eq!(run(&p, &mut fm), vec![1.0, 0.5, 2.0, 2.0]);
}
