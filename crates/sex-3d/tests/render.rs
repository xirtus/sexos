// SPDX-License-Identifier: GPL-3.0-only
//! Path tracer, denoiser and viewport tests. Kept small (low resolution, few
//! samples); the heavier convergence/benchmark runs are `#[ignore]`d — run
//! them with `cargo test --release -p sex-3d --test render -- --ignored --nocapture`.
use sex_3d::math::{Mat4, Quat, Vec3};
use sex_3d::primitives;
use sex_3d::render::viewport::{self, ViewCamera, ViewportScene, ViewportSettings};
use sex_3d::render::{self, DenoiseSettings, PreparedScene, Progressive, RenderResult, RenderSettings};
use sex_3d::scene::{Camera, Image, Light, LightKind, Material, Object, ObjectData, Scene, Transform};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

fn add_camera(s: &mut Scene, eye: Vec3, target: Vec3, fov_y: f32) {
    let mut cam = Object::new("Camera", ObjectData::Camera(Camera { fov_y, ..Camera::default() }));
    let up = if (target - eye).normalize().dot(Vec3::Z).abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    cam.transform = Transform::from_matrix(&Mat4::look_at(eye, target, up).inverse());
    let c = s.add_object(cam);
    s.active_camera = Some(c);
}

fn mean(img: &Image) -> [f64; 3] {
    let mut m = [0.0f64; 3];
    for p in &img.pixels {
        for c in 0..3 {
            m[c] += p[c] as f64;
        }
    }
    m.map(|v| v / img.pixels.len() as f64)
}

/// RMSE of display-range values (clamped to 0..1), where noise is visible.
fn rmse(a: &Image, b: &Image) -> f64 {
    let mut s = 0.0f64;
    for (p, q) in a.pixels.iter().zip(&b.pixels) {
        for c in 0..3 {
            let d = (p[c].clamp(0.0, 1.0) - q[c].clamp(0.0, 1.0)) as f64;
            s += d * d;
        }
    }
    (s / (a.pixels.len() * 3) as f64).sqrt()
}

fn settings(w: u32, h: u32, spp: u32) -> RenderSettings {
    RenderSettings { width: w, height: h, samples: spp, max_bounces: 6, threads: 0, denoise: false, clamp: 0.0, seed: 0 }
}

/// Lambertian sphere (ior 1: no specular) filling the view under a uniform world.
fn furnace_scene(albedo: f32) -> Scene {
    let mut s = Scene::new();
    s.world.color = Vec3::ONE;
    s.world.strength = 1.0;
    let m = s.add_material(Material { base_color: [albedo, albedo, albedo, 1.0], ior: 1.0, roughness: 1.0, ..Material::default() });
    let o = s.add_mesh_object("Sphere", primitives::uv_sphere(48, 24, 1.0));
    s.objects[o].materials.push(m);
    add_camera(&mut s, Vec3::new(0.0, -4.0, 0.0), Vec3::ZERO, 0.4);
    s
}

#[test]
fn white_furnace_converges_to_environment() {
    let s = furnace_scene(1.0);
    let mut st = settings(24, 24, 64);
    st.max_bounces = 64;
    let r = render::render(&s, &st);
    let m = mean(&r.color);
    for c in m {
        assert!((c - 1.0).abs() < 0.02, "furnace mean {m:?}");
    }
    // Every pixel, not only the mean: a white Lambertian under white light is invisible.
    for p in &r.color.pixels {
        assert!((p[1] - 1.0).abs() < 0.12, "pixel {p:?}");
    }
    // An albedo-0.5 sphere: a single convex surface sees only the sky, so 0.5.
    let r = render::render(&furnace_scene(0.5), &st);
    let centre = r.color.pixels[(12 * 24 + 12) as usize];
    assert!((centre[0] - 0.5).abs() < 0.05, "{centre:?}");
}

/// Closed Cornell-style box with an emissive ceiling patch, a red and a green
/// wall, a diffuse cube and a glossy metal sphere.
fn cornell() -> Scene {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let white = s.add_material(Material { base_color: [0.73, 0.73, 0.73, 1.0], roughness: 1.0, ..Material::default() });
    let red = s.add_material(Material { base_color: [0.65, 0.05, 0.05, 1.0], roughness: 1.0, ..Material::default() });
    let green = s.add_material(Material { base_color: [0.12, 0.45, 0.15, 1.0], roughness: 1.0, ..Material::default() });
    let light = s.add_material(Material { base_color: [0.0, 0.0, 0.0, 1.0], emission: Vec3::ONE, emission_strength: 12.0, ..Material::default() });
    let metal = s.add_material(Material { base_color: [0.9, 0.8, 0.6, 1.0], metallic: 1.0, roughness: 0.3, ..Material::default() });
    let wall = |s: &mut Scene, name: &str, mat: usize, loc: Vec3, rot: Quat| {
        let o = s.add_mesh_object(name, primitives::plane(2.0));
        s.objects[o].materials.push(mat);
        s.objects[o].transform = Transform { location: loc, rotation: rot, scale: Vec3::ONE };
    };
    let deg = core::f32::consts::PI / 180.0;
    wall(&mut s, "Floor", white, Vec3::ZERO, Quat::IDENTITY);
    wall(&mut s, "Ceiling", white, Vec3::new(0.0, 0.0, 2.0), Quat::from_euler_xyz(180.0 * deg, 0.0, 0.0));
    wall(&mut s, "Back", white, Vec3::new(0.0, 1.0, 1.0), Quat::from_euler_xyz(90.0 * deg, 0.0, 0.0));
    wall(&mut s, "Left", red, Vec3::new(-1.0, 0.0, 1.0), Quat::from_euler_xyz(0.0, 90.0 * deg, 0.0));
    wall(&mut s, "Right", green, Vec3::new(1.0, 0.0, 1.0), Quat::from_euler_xyz(0.0, -90.0 * deg, 0.0));
    let lamp = s.add_mesh_object("Lamp", primitives::plane(0.5));
    s.objects[lamp].materials.push(light);
    s.objects[lamp].transform = Transform { location: Vec3::new(0.0, 0.0, 1.98), rotation: Quat::from_euler_xyz(180.0 * deg, 0.0, 0.0), scale: Vec3::ONE };
    let cube = s.add_mesh_object("Box", primitives::cube(0.6));
    s.objects[cube].materials.push(white);
    s.objects[cube].transform = Transform { location: Vec3::new(-0.35, 0.3, 0.3), rotation: Quat::from_euler_xyz(0.0, 0.0, 20.0 * deg), scale: Vec3::new(1.0, 1.0, 1.0) };
    let ball = s.add_mesh_object("Ball", primitives::uv_sphere(32, 16, 0.35));
    s.objects[ball].materials.push(metal);
    s.objects[ball].transform.location = Vec3::new(0.4, -0.2, 0.35);
    add_camera(&mut s, Vec3::new(0.0, -3.4, 1.0), Vec3::new(0.0, 0.0, 1.0), 0.7);
    s
}

#[test]
fn cornell_box_converges() {
    let s = cornell();
    let low = render::render(&s, &settings(24, 24, 32));
    let mut hs = settings(24, 24, 256);
    hs.seed = 99;
    let high = render::render(&s, &hs);
    let (a, b) = (mean(&low.color), mean(&high.color));
    for c in 0..3 {
        assert!(b[c] > 0.02, "box is lit: {b:?}");
        assert!(((a[c] - b[c]) / b[c]).abs() < 0.06, "low {a:?} vs high {b:?}");
    }
    // Colour bleeding sanity: the left half is redder than the right half.
    let half = |x0: u32, x1: u32| {
        let mut r = [0.0f64; 3];
        for y in 0..24 {
            for x in x0..x1 {
                let p = high.color.pixels[(y * 24 + x) as usize];
                for c in 0..3 {
                    r[c] += p[c] as f64;
                }
            }
        }
        r
    };
    let (l, r) = (half(0, 6), half(18, 24));
    assert!(l[0] / l[1] > r[0] / r[1], "left {l:?} right {r:?}");
}

#[test]
fn region_render_is_bit_identical() {
    let s = cornell();
    let mut st = settings(20, 16, 4);
    st.threads = 3;
    let full = render::render(&s, &st);
    st.threads = 1;
    let region = render::render_region(&s, &st, 5, 3, 17, 11);
    assert_eq!((region.color.width, region.color.height), (12, 8));
    for y in 0..8 {
        for x in 0..12 {
            let a = region.color.pixels[y * 12 + x];
            let b = full.color.pixels[(y + 3) * 20 + x + 5];
            assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "pixel ({x}, {y})");
        }
    }
    // The progressive accumulator agrees with the batch render.
    st.threads = 2;
    let mut p = Progressive::new(&s, &st);
    for _ in 0..4 {
        assert!(p.add_sample(None));
    }
    assert_eq!(p.samples(), 4);
    for (a, b) in p.image().pixels.iter().zip(&full.color.pixels) {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits));
    }
}

#[test]
fn cancellation_stops_rendering() {
    let s = cornell();
    let cancel = AtomicBool::new(true);
    let r = render::render_cancellable(&s, &settings(16, 16, 4), &cancel);
    assert!(!r.complete);
    let mut p = Progressive::new(&s, &settings(16, 16, 1));
    assert!(!p.add_sample(Some(&cancel)));
    assert_eq!(p.samples(), 0);
}

#[test]
fn glass_sphere_has_no_nans() {
    let mut s = Scene::new();
    s.world.color = Vec3::new(0.8, 0.9, 1.0);
    let glass = s.add_material(Material { transmission: 1.0, roughness: 0.0, ior: 1.45, base_color: [1.0, 1.0, 1.0, 1.0], ..Material::default() });
    let rough = s.add_material(Material { transmission: 1.0, roughness: 0.4, ior: 1.5, base_color: [0.9, 1.0, 0.9, 1.0], ..Material::default() });
    let a = s.add_mesh_object("Glass", primitives::uv_sphere(32, 16, 1.0));
    s.objects[a].materials.push(glass);
    let b = s.add_mesh_object("Rough", primitives::ico_sphere(2, 0.5));
    s.objects[b].materials.push(rough);
    s.objects[b].transform.location = Vec3::new(1.4, 0.0, 0.0);
    let floor = s.add_mesh_object("Floor", primitives::plane(20.0));
    s.objects[floor].transform.location = Vec3::new(0.0, 0.0, -1.0);
    let mut sun = Light { kind: LightKind::Sun, power: 3.0, radius: 0.05, ..Light::default() };
    sun.color = Vec3::ONE;
    let l = s.add_object(Object::new("Sun", ObjectData::Light(sun)));
    s.objects[l].transform.rotation = Quat::from_euler_xyz(0.3, 0.2, 0.0);
    add_camera(&mut s, Vec3::new(0.0, -5.0, 1.0), Vec3::ZERO, 0.6);
    let mut st = settings(24, 24, 16);
    st.max_bounces = 12;
    let r = render::render(&s, &st);
    let mut lit = 0;
    for p in &r.color.pixels {
        assert!(p.iter().all(|v| v.is_finite() && *v >= 0.0), "{p:?}");
        lit += (p[1] > 0.05) as usize;
    }
    assert!(lit > 24 * 24 / 2);
    // The sphere refracts the bright sky and floor: its centre is not black.
    let c = r.color.pixels[12 * 24 + 12];
    assert!(c[1] > 0.05, "{c:?}");
}

/// Form factor from a differential area to a parallel 1 m square centred
/// 1 m above it.
const SQUARE_FORM_FACTOR: f32 = 0.2394;

#[test]
fn emissive_quad_lights_the_floor() {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let white = s.add_material(Material { base_color: [0.8, 0.8, 0.8, 1.0], ior: 1.0, ..Material::default() });
    let floor = s.add_mesh_object("Floor", primitives::plane(4.0));
    s.objects[floor].materials.push(white);
    let emit = s.add_material(Material { base_color: [0.0; 4], emission: Vec3::ONE, emission_strength: 5.0, ..Material::default() });
    let q = s.add_mesh_object("Panel", primitives::plane(1.0));
    s.objects[q].materials.push(emit);
    s.objects[q].transform.location = Vec3::new(0.0, 0.0, 1.0);
    add_camera(&mut s, Vec3::new(0.0, -3.0, 3.0), Vec3::new(0.0, 0.0, 0.0), 0.5);
    let r = render::render(&s, &settings(16, 16, 16));
    // Everything non-black came from the quad.
    let lit = r.color.pixels.iter().filter(|p| p[0] > 0.01).count();
    assert!(lit > 16 * 4, "{lit}");
    // Analytic check at the floor centre (seen from below the panel):
    // outgoing radiance = albedo * L * form factor.
    s.objects.pop();
    add_camera(&mut s, Vec3::new(0.0, 0.0, 0.5), Vec3::ZERO, 0.02);
    let mut st = settings(2, 2, 1024);
    st.max_bounces = 1;
    let m = mean(&render::render(&s, &st).color)[0] as f32;
    let expected = 0.8 * 5.0 * SQUARE_FORM_FACTOR;
    assert!((m - expected).abs() < 0.03 * expected, "{m} vs {expected}");
}

#[test]
fn lights_follow_blender_units() {
    use core::f32::consts::PI;
    // A 1000 W point light 2 m above a diffuse floor: E = P / (4 pi d^2),
    // outgoing radiance = E * albedo / pi.
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let m = s.add_material(Material { base_color: [0.5, 0.5, 0.5, 1.0], ior: 1.0, ..Material::default() });
    let f = s.add_mesh_object("Floor", primitives::plane(10.0));
    s.objects[f].materials.push(m);
    let l = s.add_object(Object::new("Lamp", ObjectData::Light(Light { radius: 0.0, ..Light::default() })));
    s.objects[l].transform.location = Vec3::new(0.0, 0.0, 2.0);
    add_camera(&mut s, Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO, 0.01);
    let mut st = settings(2, 2, 8);
    st.max_bounces = 0;
    let point = 1000.0 / (4.0 * PI * 4.0) * 0.5 / PI;
    let got = render::render(&s, &st).color.pixels[0][0];
    assert!((got - point).abs() < 0.01 * point, "{got} vs {point}");

    // A spot along its default -Z (down) matches the point light; turned up it lights nothing.
    s.objects[l].data = ObjectData::Light(Light { kind: LightKind::Spot { angle: 1.0, blend: 0.1 }, radius: 0.0, ..Light::default() });
    let got = render::render(&s, &st).color.pixels[0][0];
    assert!((got - point).abs() < 0.01 * point, "{got} vs {point}");
    s.objects[l].transform.rotation = Quat::from_euler_xyz(PI, 0.0, 0.0);
    assert!(render::render(&s, &st).color.pixels[0][0] < 1e-6);
    s.objects[l].transform.rotation = Quat::IDENTITY;

    // A 2 W/m² sun straight down: radiance 2 * 0.5 / pi.
    s.objects[l].data = ObjectData::Light(Light { kind: LightKind::Sun, power: 2.0, radius: 0.0, ..Light::default() });
    let got = render::render(&s, &st).color.pixels[0][0];
    let sun = 2.0 * 0.5 / PI;
    assert!((got - sun).abs() < 0.01 * sun, "{got} vs {sun}");

    // A 100 W, 1 m square area light 1 m up facing down: radiance P / (A pi),
    // floor radiance = albedo * L * form factor.
    s.objects[l].data = ObjectData::Light(Light { kind: LightKind::Area { size: 1.0 }, power: 100.0, radius: 0.0, ..Light::default() });
    s.objects[l].transform.location = Vec3::new(0.0, 0.0, 1.0);
    let mut st = settings(2, 2, 1024);
    st.max_bounces = 0;
    let got = mean(&render::render(&s, &st).color)[0] as f32;
    let area = 0.5 * 100.0 / PI * SQUARE_FORM_FACTOR;
    assert!((got - area).abs() < 0.03 * area, "{got} vs {area}");
}

#[test]
fn hdri_world_is_importance_sampled() {
    // A dark world with one bright patch: the floor must be lit by it, and
    // rendering must agree between a low and a high sample count.
    let mut s = Scene::new();
    let mut img = Image::new("sky", 64, 32);
    for y in 0..32 {
        for x in 0..64 {
            let v = if (4..8).contains(&y) && (20..26).contains(&x) { 400.0 } else { 0.05 };
            img.pixels[y * 64 + x] = [v, v, v, 1.0];
        }
    }
    s.images.push(img);
    s.world.hdri = Some(0);
    let f = s.add_mesh_object("Floor", primitives::plane(4.0));
    let _ = f;
    add_camera(&mut s, Vec3::new(0.0, -3.0, 2.0), Vec3::ZERO, 0.5);
    let mut st = settings(12, 12, 64);
    st.max_bounces = 1;
    let a = mean(&render::render(&s, &st).color);
    st.samples = 512;
    st.seed = 5;
    let b = mean(&render::render(&s, &st).color);
    assert!(b[0] > 0.2, "{b:?}");
    assert!(((a[0] - b[0]) / b[0]).abs() < 0.08, "{a:?} vs {b:?}");
}

#[test]
fn denoiser_lowers_error() {
    let s = cornell();
    let noisy = render::render(&s, &settings(32, 32, 8));
    let mut hs = settings(32, 32, 512);
    hs.seed = 1;
    let reference = render::render(&s, &hs);
    let den = render::denoise(&noisy, &DenoiseSettings::default());
    let (e0, e1) = (rmse(&noisy.color, &reference.color), rmse(&den, &reference.color));
    assert!(e1 < e0 * 0.7, "noisy {e0} denoised {e1}");
    // Through the settings flag too.
    let mut st = settings(32, 32, 8);
    st.denoise = true;
    let d2: RenderResult = render::render(&s, &st);
    assert_eq!(d2.color.pixels, den.pixels);
}

#[test]
fn tone_mapping_produces_rgba8() {
    let mut img = Image::new("x", 2, 1);
    img.pixels = vec![[0.0, 0.5, 1.0, 1.0], [8.0, 8.0, 8.0, 1.0]];
    let std = render::to_srgb8(&img);
    assert_eq!(std.len(), 8);
    assert_eq!(&std[0..4], &[0, 188, 255, 255]);
    assert_eq!(&std[4..8], &[255, 255, 255, 255]);
    let agx = render::to_srgb8_with(&img, render::ViewTransform::AgX, 0.0);
    assert!(agx[4] < 255 && agx[4] > 200, "AgX rolls off highlights: {}", agx[4]);
}

#[test]
fn viewport_draws_the_startup_cube() {
    let mut s = Scene::startup();
    let cube = s.find_object("Cube").unwrap();
    s.selected = vec![cube];
    let st = ViewportSettings { width: 160, height: 90, threads: 2, ..ViewportSettings::default() };
    let view = ViewCamera::default();
    let px = viewport::draw(&s, &view, &st);
    assert_eq!(px.len(), 160 * 90 * 4);
    let at = |x: usize, y: usize| &px[(y * 160 + x) * 4..(y * 160 + x) * 4 + 4];
    let corner = at(2, 2).to_vec();
    let centre = at(80, 45).to_vec();
    assert_ne!(centre[..3], corner[..3], "centre shows the cube");
    // The selected cube has an orange outline somewhere.
    let orange = px.chunks_exact(4).filter(|p| p[0] > 200 && p[1] > 100 && p[1] < 190 && p[2] < 80).count();
    assert!(orange > 20, "{orange}");
    // Wireframe overlay changes pixels; single- and multi-threaded output agree.
    let wire = viewport::draw(&s, &view, &ViewportSettings { wireframe: true, ..st });
    assert_ne!(wire, px);
    let single = viewport::draw(&s, &view, &ViewportSettings { threads: 1, ..st });
    assert_eq!(single, px);
    // Orthographic view and an inside-the-cube view do not crash and draw something.
    let ortho = viewport::draw(&s, &ViewCamera { ortho: true, ..view }, &st);
    assert_ne!(ortho[(45 * 160 + 80) * 4..][..3], corner[..3]);
    let inside = viewport::draw(&s, &ViewCamera { distance: 0.5, ..view }, &st);
    assert_eq!(inside.len(), px.len());
}

#[test]
fn picking_returns_the_cube() {
    let s = Scene::startup();
    let view = ViewCamera::default();
    let cube = s.find_object("Cube").unwrap();
    assert_eq!(viewport::pick(&s, &view, 160, 90, 80.0, 45.0), Some(cube));
    assert_eq!(viewport::pick(&s, &view, 160, 90, 1.0, 1.0), None);
    // The light gizmo is pickable near its projected origin.
    let light = s.find_object("Light").unwrap();
    let vs = ViewportScene::new(&s);
    let found = (0..90).flat_map(|y| (0..160).map(move |x| (x, y))).find_map(|(x, y)| (vs.pick(&view, 160, 90, x as f32, y as f32) == Some(light)).then_some(()));
    assert!(found.is_some());
    // The path tracer's BVH agrees.
    let p = PreparedScene::new(&s);
    let cam = s.world_matrix(s.active_camera.unwrap());
    assert_eq!(p.pick_ray(cam.transform_point(Vec3::ZERO), cam.transform_vector(-Vec3::Z)), Some(cube));
}

#[test]
fn startup_scene_renders() {
    let s = Scene::startup();
    let r = render::render(&s, &settings(32, 18, 4));
    let c = r.color.pixels[9 * 32 + 16];
    assert!(c[0] > 0.05, "the lit cube faces the camera: {c:?}");
}

fn dense_mesh_scene() -> Scene {
    let mut s = Scene::startup();
    let cube = s.find_object("Cube").unwrap();
    s.remove_object(cube);
    // ~100k triangles: a 256 x 192 smooth UV sphere.
    let o = s.add_mesh_object("Dense", primitives::uv_sphere(256, 196, 1.5));
    s.selected = vec![o];
    s.active_object = Some(o);
    s
}

#[test]
#[ignore]
fn bench_path_tracer_and_viewport() {
    let s = Scene::startup();
    let st = RenderSettings { width: 320, height: 180, samples: 32, max_bounces: 4, ..RenderSettings::default() };
    let p = PreparedScene::new(&s);
    let t = Instant::now();
    let r = p.render_region(&st, 0, 0, st.width, st.height, None);
    let dt = t.elapsed().as_secs_f64();
    let samples = (st.width * st.height * st.samples) as f64;
    println!("path tracer: startup 320x180x32 in {dt:.3}s = {:.2} Msamples/s ({} threads)", samples / dt / 1e6, st.thread_count());
    assert!(r.complete);
    let t = Instant::now();
    let _ = p.render_region(&RenderSettings { threads: 1, ..st }, 0, 0, st.width, st.height, None);
    let dt1 = t.elapsed().as_secs_f64();
    println!("path tracer: startup single thread {:.2} Msamples/s (scaling x{:.2})", samples / dt1 / 1e6, dt1 / dt);

    let d = dense_mesh_scene();
    let t = Instant::now();
    let pd = PreparedScene::new(&d);
    println!("prepare + BVH for {} tris: {:.1} ms", pd.triangle_count(), t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let _ = pd.render_region(&st, 0, 0, st.width, st.height, None);
    let dt = t.elapsed().as_secs_f64();
    println!("path tracer: 100k-tri 320x180x32 in {dt:.3}s = {:.2} Msamples/s", samples / dt / 1e6);

    // Primary-ray throughput on the dense mesh.
    let t = Instant::now();
    let mut hits = 0u64;
    let n = 1_000_000u32;
    let cam_eye = Vec3::new(7.36, -6.93, 4.96);
    for i in 0..n {
        let (u, v) = ((i % 1000) as f32 / 1000.0 - 0.5, (i / 1000) as f32 / 1000.0 - 0.5);
        let d = (Vec3::ZERO - cam_eye).normalize() + Vec3::new(u * 0.3, v * 0.3, u * v * 0.1);
        hits += pd.pick_ray(cam_eye, d).is_some() as u64;
    }
    let dt = t.elapsed().as_secs_f64();
    println!("BVH closest-hit: {:.2} Mrays/s single thread ({hits} hits)", n as f64 / dt / 1e6);

    for (name, scene) in [("startup", &s), ("100k-tri sphere", &d)] {
        let vs = ViewportScene::new(scene);
        let view = ViewCamera::default();
        for threads in [1, 0] {
            let vst = ViewportSettings { width: 1280, height: 720, threads, ..ViewportSettings::default() };
            let mut buf = Vec::new();
            vs.draw_into(&view, &vst, &mut buf);
            let t = Instant::now();
            let frames = 10;
            for _ in 0..frames {
                vs.draw_into(&view, &vst, &mut buf);
            }
            let ms = t.elapsed().as_secs_f64() * 1e3 / frames as f64;
            println!("viewport {name} 1280x720 ({} tris, threads {}): {ms:.2} ms/frame", vs.triangle_count(), vst.threads);
            let wire = ViewportSettings { wireframe: true, ..vst };
            let t = Instant::now();
            vs.draw_into(&view, &wire, &mut buf);
            println!("  with wireframe: {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
        }
        let t = Instant::now();
        let _ = ViewportScene::new(scene);
        println!("  ViewportScene::new: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    }
}

#[test]
#[ignore]
fn heavy_cornell_convergence() {
    let s = cornell();
    let low = render::render(&s, &settings(64, 64, 64));
    let mut hs = settings(64, 64, 2048);
    hs.seed = 3;
    let high = render::render(&s, &hs);
    let (a, b) = (mean(&low.color), mean(&high.color));
    for c in 0..3 {
        assert!(((a[c] - b[c]) / b[c]).abs() < 0.02, "{a:?} vs {b:?}");
    }
    let den = render::denoise(&low, &DenoiseSettings::default());
    println!("RMSE noisy {:.4} denoised {:.4}", rmse(&low.color, &high.color), rmse(&den, &high.color));
}
