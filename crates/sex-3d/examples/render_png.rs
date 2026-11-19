// SPDX-License-Identifier: GPL-3.0-only
//! Path-trace a scene file (or the startup scene) to a PNG on the host:
//! `cargo run --release -p sex-3d --example render_png -- [scene.glb|.obj|.blend] out.png [samples]`
use sex_3d::{io, render, scene::Scene};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (scene, out, rest) = match args.as_slice() {
        [inp, out, rest @ ..] if io::Format::from_path(inp).is_some() => {
            let bytes = std::fs::read(inp).expect("read scene");
            let dir = std::path::Path::new(inp).parent().map(|p| p.to_path_buf()).unwrap_or_default();
            let resolve = move |p: &str| std::fs::read(dir.join(p)).ok();
            (io::import(&bytes, io::Format::from_path(inp).unwrap(), &resolve).expect("import"), out.clone(), rest)
        }
        [out, rest @ ..] => (demo_scene(), out.clone(), rest),
        [] => (demo_scene(), "polyamour.png".to_string(), &[][..]),
    };
    let samples = rest.first().and_then(|s| s.parse().ok()).unwrap_or(64);
    let settings = render::RenderSettings { width: 960, height: 540, samples, denoise: true, ..Default::default() };
    let t = std::time::Instant::now();
    let r = render::render(&scene, &settings);
    eprintln!("{}x{} {} spp in {:.2?}", settings.width, settings.height, samples, t.elapsed());
    let rgba = render::to_srgb8_with(&r.color, render::ViewTransform::AgX, 0.0);
    let f = std::fs::File::create(&out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), settings.width, settings.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
}

/// Startup scene plus a subdivided, bevelled torus, a glass sphere and a floor.
fn demo_scene() -> Scene {
    use sex_3d::{modifiers::Modifier, primitives, scene::Material, Vec3};
    let mut s = Scene::startup();
    let floor = s.add_mesh_object("Floor", primitives::plane(40.0));
    s.objects[floor].transform.location = Vec3::new(0.0, 0.0, -1.0);
    let glass = s.add_material(Material { name: "Glass".into(), transmission: 1.0, roughness: 0.0, ..Default::default() });
    let gold = s.add_material(Material { name: "Gold".into(), base_color: [1.0, 0.71, 0.29, 1.0], metallic: 1.0, roughness: 0.25, ..Default::default() });
    let sphere = s.add_mesh_object("Sphere", primitives::ico_sphere(3, 0.8));
    s.objects[sphere].transform.location = Vec3::new(2.4, -1.2, -0.2);
    s.objects[sphere].materials.push(glass);
    let torus = s.add_mesh_object("Torus", primitives::torus(32, 12, 1.0, 0.3));
    s.objects[torus].transform.location = Vec3::new(-0.5, -2.6, -0.7);
    s.objects[torus].materials.push(gold);
    s.objects[torus].modifiers.push(Modifier::Subdivision { levels: 1, simple: false });
    s.world.color = Vec3::new(0.35, 0.4, 0.5);
    s
}
