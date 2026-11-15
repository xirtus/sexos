// SPDX-License-Identifier: GPL-3.0-only
//! Tests for the Cycles-style render features: cameras and depth of field,
//! adaptive sampling, film and passes, light shapes and options, ray
//! visibility, volumes, motion blur and bounce limits. Small and fast in
//! release builds.
use core::f32::consts::PI;
use sex_3d::math::{Mat4, Vec3};
use sex_3d::primitives;
use sex_3d::render::{self, LightPaths, MistFalloff, MistSettings, MotionBlur, Pass, Passes, RenderBorder, RenderSettings, ShutterPosition};
use sex_3d::scene::{AreaShape, Camera, Image, Light, LightKind, Material, Object, ObjectData, PanoramaType, Scene, Transform, VolumeSettings};

fn add_camera(s: &mut Scene, cam: Camera, eye: Vec3, target: Vec3) -> usize {
    let mut o = Object::new("Camera", ObjectData::Camera(cam));
    let up = if (target - eye).normalize().dot(Vec3::Z).abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    o.transform = Transform::from_matrix(&Mat4::look_at(eye, target, up).inverse());
    let c = s.add_object(o);
    s.active_camera = Some(c);
    c
}

fn settings(w: u32, h: u32, spp: u32) -> RenderSettings {
    RenderSettings { width: w, height: h, samples: spp, max_bounces: 0, clamp: 0.0, light_paths: LightPaths::unlimited(), ..RenderSettings::default() }
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

fn px(img: &Image, x: u32, y: u32) -> [f32; 4] {
    img.pixels[(y * img.width + x) as usize]
}

/// An emissive quad of `w` x `h` metres in the XY plane at `at`.
fn emitter(s: &mut Scene, name: &str, at: Vec3, w: f32, h: f32, strength: f32) -> usize {
    let m = s.add_material(Material { base_color: [0.0, 0.0, 0.0, 1.0], emission: Vec3::ONE, emission_strength: strength, ..Material::default() });
    let o = s.add_mesh_object(name, primitives::plane(1.0));
    s.objects[o].transform.location = at;
    s.objects[o].transform.scale = Vec3::new(w, h, 1.0);
    s.objects[o].materials.push(m);
    o
}

/// Columns of row `y` brighter than a tenth of the brightest in `x0..x1`.
fn lit_width(img: &Image, y: u32, x0: u32, x1: u32) -> usize {
    let row: Vec<f32> = (x0..x1).map(|x| px(img, x, y)[0]).collect();
    let max = row.iter().cloned().fold(0.0, f32::max);
    row.iter().filter(|&&v| v > 0.1 * max).count()
}

#[test]
fn depth_of_field_blurs_away_from_focus() {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    // A strip at 10 m (focused) on the left, one at 20 m on the right.
    emitter(&mut s, "Near", Vec3::new(-1.0, 0.0, 0.0), 0.1, 2.0, 5.0);
    emitter(&mut s, "Far", Vec3::new(2.0, 0.0, -10.0), 0.2, 4.0, 5.0);
    let mut cam = Camera::default();
    let c = add_camera(&mut s, cam, Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO);
    let st = settings(64, 64, 32);
    let sharp = render::render(&s, &st).color;
    let (near0, far0) = (lit_width(&sharp, 32, 0, 32), lit_width(&sharp, 32, 32, 64));
    assert!(near0 <= 4 && far0 <= 4, "pinhole: {near0} {far0}");
    cam.dof.enabled = true;
    cam.dof.focus_distance = 10.0;
    cam.dof.fstop = 0.1;
    s.objects[c].data = ObjectData::Camera(cam);
    let dof = render::render(&s, &st).color;
    let (near1, far1) = (lit_width(&dof, 32, 0, 32), lit_width(&dof, 32, 32, 64));
    assert!(near1 <= 4, "in focus stays sharp: {near1}");
    assert!(far1 >= 7, "out of focus spreads: {far1}");
    // Focusing on the far strip's object swaps them.
    let far = s.find_object("Far").unwrap();
    cam.dof.focus_object = Some(far);
    s.objects[c].data = ObjectData::Camera(cam);
    let swapped = render::render(&s, &st).color;
    assert!(lit_width(&swapped, 32, 0, 32) > near1 + 2 && lit_width(&swapped, 32, 32, 64) < far1);
    // A hexagonal aperture still blurs by about as much.
    cam.dof.focus_object = None;
    cam.dof.blades = 6;
    s.objects[c].data = ObjectData::Camera(cam);
    let hex = render::render(&s, &st).color;
    assert!(lit_width(&hex, 32, 32, 64) >= 6);
}

#[test]
fn adaptive_sampling_stops_early_on_flat_regions() {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(0.5);
    // A sphere lit by a small area light is noisy; the sky around it is flat.
    let o = s.add_mesh_object("Sphere", primitives::uv_sphere(32, 16, 1.0));
    let m = s.add_material(Material { roughness: 1.0, ..Material::default() });
    s.objects[o].materials.push(m);
    let l = s.add_object(Object::new("Area", ObjectData::Light(Light { kind: LightKind::Area { size: 0.3 }, power: 300.0, ..Light::default() })));
    s.objects[l].transform.location = Vec3::new(0.0, 0.0, 3.0);
    add_camera(&mut s, Camera { fov_y: 0.6, ..Camera::default() }, Vec3::new(0.0, -6.0, 0.0), Vec3::ZERO);
    let mut st = settings(24, 24, 256);
    st.max_bounces = 4;
    st.noise_threshold = 0.01;
    st.min_samples = 16;
    let r = render::render(&s, &st);
    assert_eq!(r.sample_counts.len(), 24 * 24);
    // Corners see only the uniform sky: they stop at the minimum.
    for (x, y) in [(0, 0), (23, 0), (0, 23), (23, 23)] {
        assert_eq!(r.sample_counts[(y * 24 + x) as usize], 16);
    }
    let centre = r.sample_counts[12 * 24 + 12];
    assert!(centre > 16, "noisy pixels keep sampling: {centre}");
    let total: u64 = r.sample_counts.iter().map(|&n| n as u64).sum();
    assert!(total < 24 * 24 * 256 / 2, "{total}");
    // Pixels stop on their own samples alone: thread count changes nothing.
    let one = render::render(&s, &RenderSettings { threads: 1, ..st });
    let many = render::render(&s, &RenderSettings { threads: 5, ..st });
    assert_eq!(one.color, many.color);
    assert_eq!(one.sample_counts, many.sample_counts);
}

#[test]
fn transparent_film_and_holdout() {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(2.0);
    let o = s.add_mesh_object("Sphere", primitives::uv_sphere(24, 12, 1.0));
    add_camera(&mut s, Camera { fov_y: 0.8, ..Camera::default() }, Vec3::new(0.0, -6.0, 0.0), Vec3::ZERO);
    let mut st = settings(16, 16, 4);
    let opaque = render::render(&s, &st).color;
    assert_eq!(px(&opaque, 0, 0)[3], 1.0);
    assert!(px(&opaque, 0, 0)[0] > 1.9);
    st.film_transparent = true;
    st.passes = Passes::NONE.with(Pass::Environment);
    let r = render::render(&s, &st);
    let bg = px(&r.color, 0, 0);
    assert_eq!(bg, [0.0, 0.0, 0.0, 0.0], "background is transparent and black");
    let centre = px(&r.color, 8, 8);
    assert_eq!(centre[3], 1.0);
    assert!(centre[0] > 0.1, "the sphere is still lit by the world");
    assert!(px(r.pass(Pass::Environment).unwrap(), 0, 0)[0] > 1.9, "the Env pass keeps the sky");
    // A holdout object cuts a hole even in an opaque film.
    s.objects[o].holdout = true;
    st.film_transparent = false;
    let h = render::render(&s, &st).color;
    assert_eq!(px(&h, 8, 8), [0.0, 0.0, 0.0, 0.0]);
    assert_eq!(px(&h, 0, 0)[3], 1.0);
}

#[test]
fn data_passes_match_the_geometry() {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let f = s.add_mesh_object("Floor", primitives::plane(4.0));
    s.objects[f].pass_index = 3;
    let m = s.add_material(Material { base_color: [0.6, 0.6, 0.6, 1.0], ior: 1.0, roughness: 1.0, pass_index: 7, ..Material::default() });
    s.objects[f].materials.push(m);
    let sun = s.add_object(Object::new("Sun", ObjectData::Light(Light { kind: LightKind::Sun, power: 3.0, radius: 0.0, ..Light::default() })));
    s.objects[sun].transform.location = Vec3::new(0.0, 0.0, 5.0);
    add_camera(&mut s, Camera { fov_y: 0.6, ..Camera::default() }, Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
    let mut st = settings(16, 16, 8);
    st.max_bounces = 2;
    st.mist = MistSettings { start: 0.0, depth: 10.0, falloff: MistFalloff::Linear };
    st.passes = [Pass::Depth, Pass::Mist, Pass::Normal, Pass::Position, Pass::ObjectIndex, Pass::MaterialIndex, Pass::DiffuseDirect, Pass::DiffuseColor, Pass::AmbientOcclusion, Pass::Uv].into_iter().collect();
    let r = render::render(&s, &st);
    let p = |pass: Pass, x, y| px(r.pass(pass).unwrap(), x, y);
    // The floor faces the camera 5 m away: view-axis depth is 5 everywhere it covers.
    for (x, y) in [(8, 8), (2, 3), (14, 12)] {
        assert!((p(Pass::Depth, x, y)[0] - 5.0).abs() < 1e-3, "{:?}", p(Pass::Depth, x, y));
        assert_eq!(p(Pass::ObjectIndex, x, y)[0], 3.0);
        assert_eq!(p(Pass::MaterialIndex, x, y)[0], 7.0);
        assert!(p(Pass::Normal, x, y)[2] > 0.99);
        assert!(p(Pass::Position, x, y)[2].abs() < 1e-3);
        assert!((p(Pass::DiffuseColor, x, y)[0] - 0.6).abs() < 1e-3);
        assert_eq!(p(Pass::AmbientOcclusion, x, y)[0], 1.0);
    }
    // Mist: distance 5 of a 10 m linear ramp (the centre pixel looks straight down).
    assert!((p(Pass::Mist, 8, 8)[0] - 0.5).abs() < 0.02, "{:?}", p(Pass::Mist, 8, 8));
    // Combined = direct diffuse light x diffuse colour for a lone lit floor.
    let c = px(&r.color, 8, 8)[0];
    let recon = p(Pass::DiffuseDirect, 8, 8)[0] * p(Pass::DiffuseColor, 8, 8)[0];
    assert!((c - recon).abs() < 0.02 * c, "{c} vs {recon}");
    assert!((c - 3.0 * 0.6 / PI).abs() < 0.02, "{c}");
    // Outside the floor: background depth and index 0.
    let wide = RenderSettings { width: 32, height: 32, ..st };
    let mut s2 = s.clone();
    s2.objects[f].transform.scale = Vec3::splat(0.25);
    let r2 = render::render(&s2, &wide);
    assert_eq!(px(r2.pass(Pass::Depth).unwrap(), 0, 0)[0], render::BACKGROUND_DEPTH);
    assert_eq!(px(r2.pass(Pass::ObjectIndex).unwrap(), 0, 0)[0], 0.0);
    assert_eq!(px(r2.pass(Pass::Mist).unwrap(), 0, 0)[0], 1.0);
}

#[test]
fn area_light_shapes_conserve_energy() {
    // Far from the light, every shape of the same power lights the floor alike.
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let m = s.add_material(Material { base_color: [0.5, 0.5, 0.5, 1.0], ior: 1.0, ..Material::default() });
    let f = s.add_mesh_object("Floor", primitives::plane(2.0));
    s.objects[f].materials.push(m);
    let l = s.add_object(Object::new("Area", ObjectData::Light(Light::default())));
    s.objects[l].transform.location = Vec3::new(0.0, 0.0, 4.0);
    add_camera(&mut s, Camera { fov_y: 0.05, ..Camera::default() }, Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO);
    let st = settings(4, 4, 512);
    let mut got = Vec::new();
    for (shape, size_y) in [(AreaShape::Square, 1.0), (AreaShape::Rectangle, 0.5), (AreaShape::Disk, 1.0), (AreaShape::Ellipse, 0.4)] {
        s.objects[l].data = ObjectData::Light(Light { kind: LightKind::Area { size: 0.8 }, shape, size_y, power: 200.0, ..Light::default() });
        got.push(mean(&render::render(&s, &st).color)[0] as f32);
    }
    // E ~ P / (pi d^2) on axis for a small Lambertian emitter: floor L = 0.5 E / pi.
    let expected = 0.5 * 200.0 / (PI * 16.0) / PI;
    for g in &got {
        assert!((g - expected).abs() < 0.04 * expected, "{got:?} vs {expected}");
    }
    // A narrow spread concentrates the same power on axis and darkens off axis.
    s.objects[l].data = ObjectData::Light(Light { kind: LightKind::Area { size: 0.8 }, spread: 30f32.to_radians(), power: 200.0, ..Light::default() });
    let on_axis = mean(&render::render(&s, &st).color)[0] as f32;
    assert!(on_axis > 2.0 * expected, "{on_axis}");
    s.objects[l].transform.location = Vec3::new(4.0, 0.0, 4.0);
    let off_axis = mean(&render::render(&s, &st).color)[0] as f32;
    assert!(off_axis < 1e-4, "{off_axis}");
}

#[test]
fn light_options_and_ray_visibility() {
    // A floor under a sun, with a roof in between, seen from below the roof.
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let m = s.add_material(Material { base_color: [0.5, 0.5, 0.5, 1.0], ior: 1.0, ..Material::default() });
    let f = s.add_mesh_object("Floor", primitives::plane(10.0));
    s.objects[f].materials.push(m);
    let roof = s.add_mesh_object("Roof", primitives::plane(10.0));
    s.objects[roof].transform.location = Vec3::new(0.0, 0.0, 3.0);
    s.objects[roof].materials.push(m);
    let sun = s.add_object(Object::new("Sun", ObjectData::Light(Light { kind: LightKind::Sun, power: 2.0, radius: 0.0, ..Light::default() })));
    add_camera(&mut s, Camera { fov_y: 0.2, ..Camera::default() }, Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO);
    let st = settings(4, 4, 4);
    let lit = 2.0 * 0.5 / PI;
    let floor = |s: &Scene| mean(&render::render(s, &st).color)[0] as f32;
    assert!(floor(&s) < 1e-6, "the roof shadows the floor");
    // Hidden from shadow rays: the floor is lit.
    s.objects[roof].visibility.shadow = false;
    assert!((floor(&s) - lit).abs() < 0.01 * lit);
    s.objects[roof].visibility.shadow = true;
    // A light that casts no shadows lights through it.
    s.objects[sun].data = ObjectData::Light(Light { kind: LightKind::Sun, power: 2.0, radius: 0.0, cast_shadow: false, ..Light::default() });
    assert!((floor(&s) - lit).abs() < 0.01 * lit);
    // Hidden from the camera, from above: the camera sees through the roof
    // to the (shadowed) floor.
    s.objects[sun].data = ObjectData::Light(Light { kind: LightKind::Sun, power: 2.0, radius: 0.0, ..Light::default() });
    let cam = s.active_camera.unwrap();
    s.objects[cam].transform = Transform::from_matrix(&Mat4::look_at(Vec3::new(0.0, 0.0, 6.0), Vec3::ZERO, Vec3::Y).inverse());
    let top = floor(&s);
    assert!((top - lit).abs() < 0.01 * lit, "the camera sees the lit roof: {top}");
    s.objects[roof].visibility.camera = false;
    let r = render::render(&s, &RenderSettings { passes: Passes::NONE.with(Pass::Depth), ..st });
    assert!(mean(&r.color)[0] < 1e-6);
    assert!((px(r.pass(Pass::Depth).unwrap(), 2, 2)[0] - 6.0).abs() < 1e-3, "depth is the floor's");
}

#[test]
fn bounce_limits_follow_cycles() {
    // Inside a closed emissive sphere of albedo a, radiance is
    // E (1 + a + ... + a^(D+1)) with D diffuse bounces allowed.
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let m = s.add_material(Material { base_color: [0.5, 0.5, 0.5, 1.0], ior: 1.0, roughness: 1.0, emission: Vec3::ONE, emission_strength: 1.0, ..Material::default() });
    let o = s.add_mesh_object("Room", primitives::uv_sphere(24, 12, 3.0));
    s.objects[o].materials.push(m);
    add_camera(&mut s, Camera { fov_y: 1.0, ..Camera::default() }, Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
    let mut st = settings(8, 8, 128);
    st.max_bounces = 64;
    for (d, expected) in [(0u32, 1.5f32), (1, 1.75), (64, 2.0)] {
        st.light_paths.diffuse_bounces = d;
        let got = mean(&render::render(&s, &st).color)[0] as f32;
        assert!((got - expected).abs() < 0.03 * expected, "D={d}: {got} vs {expected}");
    }
    // The total limit applies on top.
    st.max_bounces = 0;
    let got = mean(&render::render(&s, &st).color)[0] as f32;
    assert!((got - 1.5).abs() < 0.045, "{got}");
}

#[test]
fn homogeneous_volume_follows_beer_lambert() {
    // A 2 m absorbing cube in front of a uniform white sky.
    let mut s = Scene::new();
    s.world.color = Vec3::ONE;
    let vol = s.add_material(Material { volume: Some(VolumeSettings { color: Vec3::ZERO, density: 0.5, absorption_color: Vec3::ZERO, ..VolumeSettings::default() }), ..Material::default() });
    let c = s.add_mesh_object("Smoke", primitives::cube(2.0));
    s.objects[c].materials.push(vol);
    add_camera(&mut s, Camera { fov_y: 0.05, ..Camera::default() }, Vec3::new(0.0, -6.0, 0.0), Vec3::ZERO);
    let mut st = settings(4, 4, 1024);
    st.max_bounces = 4;
    let got = mean(&render::render(&s, &st).color)[0] as f32;
    let expected = (-0.5f32 * 2.0).exp();
    assert!((got - expected).abs() < 0.03 * expected, "{got} vs {expected}");

    // A purely scattering white medium under a white sky conserves energy.
    s.materials[vol].volume = Some(VolumeSettings { color: Vec3::ONE, density: 1.0, ..VolumeSettings::default() });
    st.max_bounces = 128;
    st.samples = 128;
    let got = mean(&render::render(&s, &st).color)[0] as f32;
    assert!((got - 1.0).abs() < 0.05, "white furnace through fog: {got}");
    // Volume bounces 0 (Blender's default) is single scattering: darker.
    st.light_paths.volume_bounces = 0;
    let single = mean(&render::render(&s, &st).color)[0] as f32;
    assert!(single < got - 0.05, "{single} vs {got}");

    // Emission alone: L = Le * (1 - e^-st d) / st, with st = absorption.
    s.materials[vol].volume = Some(VolumeSettings { color: Vec3::ZERO, density: 0.5, emission_strength: 2.0, ..VolumeSettings::default() });
    s.world.color = Vec3::ZERO;
    let got = mean(&render::render(&s, &st).color)[0] as f32;
    let expected = 2.0 * (1.0 - (-1.0f32).exp()) / 0.5;
    assert!((got - expected).abs() < 0.03 * expected, "{got} vs {expected}");
}

#[test]
fn world_volume_attenuates_distance() {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    s.world.volume = Some(VolumeSettings { color: Vec3::ZERO, density: 0.1, ..VolumeSettings::default() });
    emitter(&mut s, "Wall", Vec3::ZERO, 4.0, 4.0, 1.0);
    add_camera(&mut s, Camera { fov_y: 0.05, ..Camera::default() }, Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
    let got = mean(&render::render(&s, &settings(4, 4, 512)).color)[0] as f32;
    let expected = (-0.5f32).exp();
    assert!((got - expected).abs() < 0.03, "{got} vs {expected}");
}

#[test]
fn orthographic_size_does_not_change_with_distance() {
    let coverage = |dist: f32| {
        let mut s = Scene::new();
        s.world.strength = 0.0;
        s.add_mesh_object("Card", primitives::plane(1.0));
        add_camera(&mut s, Camera { ortho_scale: Some(2.0), far: 100.0, ..Camera::default() }, Vec3::new(0.0, 0.0, dist), Vec3::ZERO);
        let st = RenderSettings { film_transparent: true, ..settings(32, 32, 4) };
        let r = render::render(&s, &st).color;
        r.pixels.iter().map(|p| p[3] as f64).sum::<f64>() / r.pixels.len() as f64
    };
    let (a, b) = (coverage(3.0), coverage(30.0));
    assert!((a - 0.25).abs() < 0.03 && (b - 0.25).abs() < 0.03, "{a} {b}");
}

#[test]
fn panoramic_and_shifted_cameras() {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    // The camera looks along world -Y with Z up, so its right is -X: one
    // bright card ahead, one to the right.
    let ahead = emitter(&mut s, "Ahead", Vec3::new(0.0, -5.0, 0.0), 1.0, 1.0, 1.0);
    s.objects[ahead].transform.rotation = sex_3d::Quat::from_euler_xyz(PI / 2.0, 0.0, 0.0);
    let right = emitter(&mut s, "Right", Vec3::new(-5.0, 0.0, 0.0), 1.0, 1.0, 1.0);
    s.objects[right].transform.rotation = sex_3d::Quat::from_euler_xyz(0.0, PI / 2.0, 0.0);
    let cam = Camera { panorama: Some(PanoramaType::Equirectangular), ..Camera::default() };
    let c = add_camera(&mut s, cam, Vec3::ZERO, Vec3::new(0.0, -1.0, 0.0));
    let st = settings(64, 32, 4);
    let img = render::render(&s, &st).color;
    // Centre column = straight ahead; three quarters across = 90 degrees right.
    assert!(px(&img, 32, 16)[0] > 0.5, "ahead");
    assert!(px(&img, 48, 16)[0] > 0.5, "right");
    assert!(px(&img, 16, 16)[0] < 1e-6 && px(&img, 32, 2)[0] < 1e-6);

    // Fisheye: ahead is the centre; the corners are outside the image circle.
    s.objects[c].data = ObjectData::Camera(Camera { panorama: Some(PanoramaType::FisheyeEquidistant), ..Camera::default() });
    let st = RenderSettings { film_transparent: true, ..settings(32, 32, 4) };
    let fe = render::render(&s, &st).color;
    assert!(px(&fe, 16, 16)[0] > 0.9);
    assert_eq!(px(&fe, 0, 0)[3], 0.0);
    // 180 degrees across: the right card sits at the circle's right edge.
    assert!((26..32).any(|x| px(&fe, x, 16)[0] > 0.5));
    s.objects[c].data = ObjectData::Camera(Camera { panorama: Some(PanoramaType::FisheyeEquisolid), fisheye_lens: 10.5, ..Camera::default() });
    let es = render::render(&s, &st).color;
    assert!(px(&es, 16, 16)[0] > 0.3, "{:?}", px(&es, 16, 16));
    assert_eq!(px(&es, 0, 0)[3], 0.0);

    // Lens shift moves the image without turning the camera.
    s.objects[c].data = ObjectData::Camera(Camera { fov_y: 0.6, ..Camera::default() });
    let st = settings(32, 32, 4);
    let centred = render::render(&s, &st).color;
    assert!(px(&centred, 16, 16)[0] > 0.9);
    s.objects[c].data = ObjectData::Camera(Camera { fov_y: 0.6, shift_x: 0.5, ..Camera::default() });
    let shifted = render::render(&s, &st).color;
    // Shift 0.5 moves the card half a frame width to the left.
    assert!(px(&shifted, 16, 16)[0] < 1e-6 && px(&shifted, 0, 16)[0] > 0.5);

    // Clip start beyond the card hides it.
    s.objects[c].data = ObjectData::Camera(Camera { fov_y: 0.6, near: 6.0, ..Camera::default() });
    assert!(px(&render::render(&s, &st).color, 16, 16)[0] < 1e-6);
}

#[test]
fn motion_blur_smears_moving_objects() {
    let mut s = Scene::new();
    s.world.strength = 0.0;
    let o = emitter(&mut s, "Card", Vec3::new(-1.0, 0.0, 0.0), 0.5, 0.5, 1.0);
    let mut end = s.objects[o].transform;
    end.location = Vec3::new(1.0, 0.0, 0.0);
    s.objects[o].motion = Some(end);
    add_camera(&mut s, Camera { ortho_scale: Some(4.0), ..Camera::default() }, Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
    let mut st = settings(32, 32, 32);
    let still = render::render(&s, &st).color;
    st.motion_blur = Some(MotionBlur { shutter: 1.0, position: ShutterPosition::Start });
    let blurred = render::render(&s, &st).color;
    let (w0, w1) = (lit_width(&still, 16, 0, 32), lit_width(&blurred, 16, 0, 32));
    assert!(w0 <= 6, "{w0}");
    assert!(w1 >= 18, "the card sweeps 2 m (16 px): {w1}");
    // The same energy, spread out.
    let (e0, e1) = (mean(&still)[0], mean(&blurred)[0]);
    assert!((e0 - e1).abs() < 0.1 * e0, "{e0} {e1}");
}

#[test]
fn border_crops_and_animation_helper() {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(0.3);
    add_camera(&mut s, Camera::default(), Vec3::new(0.0, -5.0, 0.0), Vec3::ZERO);
    let mut st = settings(40, 20, 2);
    st.border = Some(RenderBorder { min_x: 0.25, min_y: 0.5, max_x: 0.75, max_y: 1.0, crop: true });
    let r = render::render(&s, &st);
    assert_eq!((r.color.width, r.color.height), (20, 10));
    st.border = Some(RenderBorder { crop: false, ..st.border.unwrap() });
    let full = render::render(&s, &st);
    assert_eq!((full.color.width, full.color.height), (40, 20));
    assert_eq!(px(&full.color, 0, 0)[3], 0.0);
    assert!(px(&full.color, 20, 2)[0] > 0.29 && px(&full.color, 20, 15)[3] == 0.0);

    // Animation: one callback per frame, the animated seed changes the noise.
    st.border = None;
    st.animated_seed = true;
    let mut frames = Vec::new();
    let n = render::render_frames(1..=3, &st, |_| s.clone(), |f, r| {
        frames.push((f, r.color));
        true
    }, None);
    assert_eq!(n, 3);
    assert_eq!(frames.iter().map(|f| f.0).collect::<Vec<_>>(), vec![1, 2, 3]);
    let (a, b) = (RenderSettings { frame: 1, ..st }.effective_seed(), RenderSettings { frame: 2, ..st }.effective_seed());
    assert_ne!(a, b);
    assert_eq!(RenderSettings { animated_seed: false, frame: 9, ..st }.effective_seed(), st.seed);
    let stop = render::render_frames(1..=3, &st, |_| s.clone(), |_, _| false, None);
    assert_eq!(stop, 1);
}

#[test]
fn regions_and_progressive_match_with_passes() {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(0.4);
    let o = s.add_mesh_object("Sphere", primitives::uv_sphere(16, 8, 1.0));
    s.objects[o].pass_index = 2;
    add_camera(&mut s, Camera { fov_y: 0.8, ..Camera::default() }, Vec3::new(0.0, -5.0, 0.0), Vec3::ZERO);
    let mut st = settings(20, 12, 3);
    st.max_bounces = 3;
    st.passes = [Pass::Depth, Pass::ObjectIndex, Pass::Normal, Pass::DiffuseIndirect].into_iter().collect();
    let full = render::render(&s, &st);
    let region = render::render_region(&s, &st, 5, 3, 17, 11);
    for y in 0..8 {
        for x in 0..12 {
            assert_eq!(px(&region.color, x, y), px(&full.color, x + 5, y + 3));
            for p in st.passes.iter() {
                assert_eq!(px(region.pass(p).unwrap(), x, y), px(full.pass(p).unwrap(), x + 5, y + 3), "{p:?}");
            }
        }
    }
    let mut prog = render::Progressive::new(&s, &st);
    for _ in 0..3 {
        prog.add_sample(None);
    }
    let pr = prog.result();
    assert_eq!(pr.color, full.color);
    assert_eq!(pr.pass(Pass::ObjectIndex), full.pass(Pass::ObjectIndex));
}

#[test]
fn render_properties_round_trip_through_polyops() {
    let mut s = Scene::startup();
    let cam = s.active_camera.unwrap();
    let cube = s.find_object("Cube").unwrap();
    let light = s.find_object("Light").unwrap();
    if let ObjectData::Camera(c) = &mut s.objects[cam].data {
        c.panorama = Some(PanoramaType::FisheyeEquisolid);
        c.fisheye_lens = 8.0;
        c.sensor_fit = sex_3d::scene::SensorFit::Vertical;
        c.shift_x = 0.1;
        c.dof.enabled = true;
        c.dof.fstop = 1.4;
        c.dof.blades = 6;
        c.dof.ratio = 1.5;
        c.dof.focus_object = Some(cube);
    }
    s.objects[light].data = ObjectData::Light(Light { kind: LightKind::Area { size: 2.0 }, shape: AreaShape::Ellipse, size_y: 0.5, spread: 1.0, max_bounces: 3, cast_shadow: false, use_mis: false, ..Light::default() });
    s.objects[cube].visibility.glossy = false;
    s.objects[cube].visibility.shadow = false;
    s.objects[cube].holdout = true;
    s.objects[cube].pass_index = 4;
    let mut end = s.objects[cube].transform;
    end.location = Vec3::new(1.0, 2.0, 3.0);
    s.objects[cube].motion = Some(end);
    s.materials[0].pass_index = 9;
    s.materials[0].volume = Some(VolumeSettings { density: 0.3, anisotropy: 0.2, ..VolumeSettings::default() });
    s.world.volume = Some(VolumeSettings { emission_strength: 0.5, ..VolumeSettings::default() });
    let text = sex_3d::script::scene_to_polyops(&s);
    let mut back = Scene::new();
    sex_3d::script::run(&mut back, &text).unwrap();
    assert_eq!(back.objects, s.objects);
    assert_eq!(back.materials, s.materials);
    assert_eq!(back.world, s.world);
    // A plain scene writes none of it.
    let plain = sex_3d::script::scene_to_polyops(&Scene::startup());
    for word in ["visible_", "holdout", "volume", "shape=", "dof=", "pass_index"] {
        assert!(!plain.contains(word), "{word}");
    }
}
