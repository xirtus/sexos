// SPDX-License-Identifier: GPL-3.0-only
//! Viewport frame times on the host:
//! `cargo run --release -p sex-3d --example viewport_bench [frames]`
//!
//! Solid (the Workbench rasterizer) and Material Preview at 1280x720 on the
//! startup scene and on a ~1M-triangle scene (16 smooth spheres and a floor).
//! Material Preview is measured as the viewport runs it: the progressive
//! path tracer (one sample per frame at a quarter of the resolution, 16
//! samples to a finished image) and the rasterized engine (one TAA sample
//! per frame at full resolution).
use sex_3d::nodes::NodeTree;
use sex_3d::render::viewport::{ViewCamera, ViewportScene, ViewportSettings};
use sex_3d::render::{LightPaths, Progressive, RenderSettings};
use sex_3d::scene::{Camera, Material, Object, ObjectData, Scene};
use sex_3d::{primitives, Mat4, Vec3};
use std::time::Instant;

fn with_node_materials(mut s: Scene) -> Scene {
    for m in &mut s.materials {
        if m.node_tree.is_none() {
            m.node_tree = Some(NodeTree::default_material(&m.name));
        }
    }
    s
}

fn startup() -> Scene {
    let mut s = Scene::startup();
    let cube = s.find_object("Cube").unwrap();
    s.selected = vec![cube];
    with_node_materials(s)
}

/// 16 UV spheres of 256 x 128 segments (~65k triangles each) and a floor.
fn million() -> Scene {
    let mut s = Scene::startup();
    let cube = s.find_object("Cube").unwrap();
    s.remove_object(cube);
    let mat = s.add_material(Material { name: "Clay".into(), base_color: [0.7, 0.45, 0.3, 1.0], roughness: 0.4, ..Default::default() });
    let mesh = s.add_mesh("Sphere", primitives::uv_sphere(256, 128, 0.9));
    for i in 0..16 {
        let o = s.add_object(Object::new("Sphere", ObjectData::Mesh(mesh)));
        s.objects[o].transform.location = Vec3::new((i % 4) as f32 * 2.0 - 3.0, (i / 4) as f32 * 2.0 - 3.0, 0.0);
        s.objects[o].materials.push(mat);
        for f in &mut s.meshes[mesh].faces {
            f.smooth = true;
        }
    }
    let floor = s.add_mesh_object("Floor", primitives::plane(20.0));
    s.objects[floor].transform.location = Vec3::new(0.0, 0.0, -0.9);
    s.selected = vec![floor];
    with_node_materials(s)
}

/// The scene seen from `view` (what the viewport's render view does).
fn view_scene(scene: &Scene, view: &ViewCamera) -> Scene {
    let mut s = scene.clone();
    let (r, u, b) = (view.right(), view.up(), view.back());
    let eye = view.eye();
    let m = Mat4::from_cols_array([r.x, r.y, r.z, 0.0, u.x, u.y, u.z, 0.0, b.x, b.y, b.z, 0.0, eye.x, eye.y, eye.z, 1.0]);
    let cam = Camera { fov_y: view.fov_y, near: 0.01, far: 1e5, ..Camera::default() };
    let mut o = Object::new("__view__", ObjectData::Camera(cam));
    o.transform = sex_3d::Transform::from_matrix(&m);
    let ci = s.add_object(o);
    s.active_camera = Some(ci);
    s.world.color = Vec3::splat(0.45);
    s.world.strength = 1.0;
    s
}

fn time_ms(frames: u32, mut f: impl FnMut()) -> f64 {
    f();
    let t = Instant::now();
    for _ in 0..frames {
        f();
    }
    t.elapsed().as_secs_f64() * 1e3 / frames as f64
}

fn main() {
    let frames: u32 = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(10);
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("threads: {threads}");
    let (w, h) = (1280u32, 720u32);
    for (name, scene) in [("startup", startup()), ("1M tris", million())] {
        let view = ViewCamera::default();
        let t = Instant::now();
        let vs = ViewportScene::new(&scene);
        println!("[{name}] {} triangles, ViewportScene::new {:.1} ms", vs.triangle_count(), t.elapsed().as_secs_f64() * 1e3);
        let mut buf = Vec::new();
        for th in [1, 0] {
            let st = ViewportSettings { width: w, height: h, threads: th, ..ViewportSettings::default() };
            let ms = time_ms(frames, || vs.draw_into(&view, &st, &mut buf));
            println!("[{name}] Solid {w}x{h} threads {}: {ms:.2} ms/frame", if th == 0 { threads } else { th });
        }
        extra_solid(&vs, &view, w, h, frames, name);

        // Material Preview: the progressive path tracer, as the viewport ran it.
        let vsc = view_scene(&scene, &view);
        let rs = RenderSettings { width: w / 4, height: h / 4, samples: 1, max_bounces: 2, denoise: false, light_paths: LightPaths::unlimited(), ..Default::default() };
        let t = Instant::now();
        let mut prog = Progressive::new(&vsc, &rs);
        let prep = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        for _ in 0..16 {
            prog.add_sample(None);
            let _ = sex_3d::render::to_display8(&prog.image(), &Default::default());
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / 16.0;
        println!("[{name}] Material Preview (path tracer {}x{}, 1 spp/frame): prepare {prep:.1} ms, {ms:.2} ms/frame, {:.0} ms to 16 samples", w / 4, h / 4, ms * 16.0);
        extra_material(&scene, &view, w, h, frames, name);
    }
}

/// Solid with the heavier Workbench options.
fn extra_solid(vs: &ViewportScene, view: &ViewCamera, w: u32, h: u32, frames: u32, name: &str) {
    use sex_3d::render::workbench::{Lighting, Workbench};
    let mut buf = Vec::new();
    let opts = [
        ("MatCap", Workbench { lighting: Lighting::MatCap, ..Workbench::viewport_defaults() }),
        ("shadow + cavity", Workbench { shadow: true, cavity: true, ..Workbench::viewport_defaults() }),
        ("X-ray", Workbench { xray: true, ..Workbench::viewport_defaults() }),
    ];
    for (label, wb) in opts {
        let st = ViewportSettings { width: w, height: h, threads: 0, workbench: wb, ..ViewportSettings::default() };
        let ms = time_ms(frames, || vs.draw_into(view, &st, &mut buf));
        println!("[{name}] Solid {label} {w}x{h}: {ms:.2} ms/frame");
    }
}

/// Material Preview with the rasterized engine (look dev): one TAA sample
/// per frame at full resolution.
fn extra_material(scene: &Scene, view: &ViewCamera, w: u32, h: u32, frames: u32, name: &str) {
    use sex_3d::render::{Eevee, EeveeScene, EeveeSettings, RasterCamera};
    let t = Instant::now();
    let es = EeveeScene::new(scene, true);
    let prep = t.elapsed().as_secs_f64() * 1e3;
    let st = EeveeSettings { width: w, height: h, samples: frames + 1, lookdev: true, ..EeveeSettings::default() };
    let mut e = Eevee::new(es, &st);
    e.set_view(RasterCamera::from_view(view, w, h));
    let t = Instant::now();
    e.add_sample();
    let first = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    for _ in 0..frames {
        e.add_sample();
    }
    let ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    println!("[{name}] Material Preview (EEVEE {w}x{h}, 1 sample/frame): prepare {prep:.1} ms, first sample {first:.1} ms, {ms:.2} ms/frame, {:.0} ms to 16 samples", first + ms * 15.0);

    // Rendered EEVEE with the screen-space effects on: reflections, bloom,
    // depth of field (gather filter) and soft shadows from the scene's light.
    let es = EeveeScene::new(scene, false);
    let lens = sex_3d::render::Lens { aperture: 0.05, focus: view.distance };
    let st = EeveeSettings { width: w, height: h, samples: frames + 1, ssr: true, bloom: true, lens: Some(lens), ..EeveeSettings::default() };
    let mut e = Eevee::new(es, &st);
    e.set_view(RasterCamera::from_view(view, w, h));
    e.add_sample();
    let t = Instant::now();
    for _ in 0..frames {
        e.add_sample();
    }
    let ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
    println!("[{name}] Rendered (EEVEE {w}x{h}, raytracing + bloom + DOF): {ms:.2} ms/frame");
}
