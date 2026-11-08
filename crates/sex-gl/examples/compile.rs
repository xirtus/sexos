//! Compiles a GLSL ES shader file the way WebGL does and prints the log, or with `-t` the
//! GLSL 450 the translation produced: `cargo run -p sex-gl --example compile -- [-t] vert|frag FILE`
use sex_gl::glsl::{translate, Stage};
fn main() {
    let mut a: Vec<String> = std::env::args().skip(1).collect();
    let show = a.first().map_or(false, |x| x == "-t");
    if show { a.remove(0); }
    let stage = if a[0].starts_with('v') { Stage::Vertex } else { Stage::Fragment };
    let src = std::fs::read_to_string(&a[1]).unwrap();
    if show {
        match translate(&src, stage, &mut Vec::new()) {
            Ok(t) => for (i, l) in t.source.lines().enumerate() { println!("{:4} {l}", i + 1) },
            Err(e) => println!("translate: {e}"),
        }
    }
    let s = sex_gl::program::Shader::compile(stage, &src);
    println!("ok={} {}", s.ok, s.log);
}
