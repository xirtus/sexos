// SPDX-License-Identifier: GPL-3.0-only
//! The rasterized engines: Solid / Workbench (the tiled rasterizer, its
//! options and the Workbench render engine) and EEVEE (deferred node
//! shading, lights and shadow maps, world prefiltering, transparency).
use sex_3d::math::{Mat4, Vec3};
use sex_3d::primitives;
use sex_3d::render::viewport::{self, ViewCamera, ViewportScene, ViewportSettings};
use sex_3d::render::workbench::{CavityType, ColorType, Lighting, Workbench, WorkbenchRender};
use sex_3d::scene::{Camera, Material, Object, ObjectData, Scene, Transform};

fn load_png(path: &str) -> (u32, u32, Vec<u8>) {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).expect("reference image")));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size().unwrap()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

/// (mean absolute difference per channel, fraction of pixels off by more than 24).
fn image_diff(a: &[u8], b: &[u8]) -> (f64, f64) {
    assert_eq!(a.len(), b.len());
    let mut sum = 0u64;
    let mut bad = 0usize;
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let d = (0..3).map(|c| (p[c] as i32 - q[c] as i32).unsigned_abs()).max().unwrap();
        sum += (0..3).map(|c| (p[c] as i32 - q[c] as i32).unsigned_abs() as u64).sum::<u64>();
        bad += (d > 24) as usize;
    }
    (sum as f64 / (a.len() / 4 * 3) as f64, bad as f64 / (a.len() / 4) as f64)
}

fn add_camera(s: &mut Scene, eye: Vec3, target: Vec3, fov_y: f32) -> usize {
    let mut cam = Object::new("Camera", ObjectData::Camera(Camera { fov_y, ..Camera::default() }));
    let up = if (target - eye).normalize().dot(Vec3::Z).abs() > 0.99 { Vec3::Y } else { Vec3::Z };
    cam.transform = Transform::from_matrix(&Mat4::look_at(eye, target, up).inverse());
    let c = s.add_object(cam);
    s.active_camera = Some(c);
    c
}

/// The startup scene plus a smooth red monkey and a floor, both selected.
fn monkey_scene() -> Scene {
    let mut s = Scene::startup();
    let cube = s.find_object("Cube").unwrap();
    let red = s.add_material(Material { base_color: [0.8, 0.1, 0.05, 1.0], ..Default::default() });
    let mut m = primitives::monkey();
    for f in &mut m.faces {
        f.smooth = true;
    }
    let o = s.add_mesh_object("Suzanne", m);
    s.objects[o].materials.push(red);
    s.objects[o].transform.location = Vec3::new(0.0, -2.5, 1.5);
    let p = s.add_mesh_object("Plane", primitives::plane(8.0));
    s.objects[p].transform.location = Vec3::new(0.0, 0.0, -1.0);
    s.selected = vec![o, cube];
    s.active_object = Some(o);
    s
}

fn monkey_view() -> ViewCamera {
    ViewCamera { yaw: -1.2, pitch: 0.35, distance: 11.0, ..ViewCamera::default() }
}

/// The Solid view looks as it did before the tiled rasterizer (reference
/// images drawn by the previous band rasterizer), and the output does not
/// depend on the thread count.
#[test]
fn solid_matches_reference_images() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data");
    let mut s = Scene::startup();
    let cube = s.find_object("Cube").unwrap();
    s.selected = vec![cube];
    let st = ViewportSettings { width: 320, height: 180, threads: 1, ..ViewportSettings::default() };
    let got = ViewportScene::new(&s).draw(&ViewCamera::default(), &st);
    let (w, h, want) = load_png(&format!("{dir}/solid_startup.png"));
    assert_eq!((w, h), (320, 180));
    let (mean, bad) = image_diff(&got, &want);
    assert!(mean < 0.5 && bad < 0.003, "startup: mean {mean:.3}, {:.3}% pixels off", bad * 100.0);
    assert_eq!(ViewportScene::new(&s).draw(&ViewCamera::default(), &ViewportSettings { threads: 7, ..st }), got);

    let s = monkey_scene();
    let st = ViewportSettings { width: 320, height: 180, threads: 1, background: Some([0.24; 3]), ..ViewportSettings::default() };
    let vs = ViewportScene::new(&s);
    let got = vs.draw(&monkey_view(), &st);
    let (_, _, want) = load_png(&format!("{dir}/solid_monkey.png"));
    let (mean, bad) = image_diff(&got, &want);
    assert!(mean < 0.5 && bad < 0.003, "monkey: mean {mean:.3}, {:.3}% pixels off", bad * 100.0);
    for threads in [2, 3, 16] {
        assert_eq!(vs.draw(&monkey_view(), &ViewportSettings { threads, ..st }), got, "{threads} threads");
    }
}

/// Every Workbench option draws, deterministically across thread counts.
#[test]
fn workbench_options_are_thread_independent() {
    let mut s = monkey_scene();
    // A colour attribute and a textured material.
    let img = s.images.len();
    s.images.push(sex_3d::scene::Image { name: "checker".into(), width: 8, height: 8, pixels: (0..64).map(|i| if (i % 8 + i / 8) % 2 == 0 { [1.0, 1.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] }).collect() });
    let cube = s.find_object("Cube").unwrap();
    let tm = s.add_material(Material { base_color_texture: Some(img), ..Default::default() });
    s.objects[cube].materials = vec![tm];
    let view = monkey_view();
    let base = ViewportSettings { width: 200, height: 120, threads: 1, ..ViewportSettings::default() };
    let vs = ViewportScene::new(&s);
    let plain = vs.draw(&view, &base);
    let variants = [
        Workbench { lighting: Lighting::Flat, ..Workbench::default() },
        Workbench { lighting: Lighting::MatCap, matcap: 1, ..Workbench::default() },
        Workbench { color: ColorType::Random, ..Workbench::default() },
        Workbench { color: ColorType::Single, single_color: [0.1, 0.9, 0.1], ..Workbench::default() },
        Workbench { color: ColorType::Texture, ..Workbench::default() },
        Workbench { color: ColorType::Attribute, ..Workbench::default() },
        Workbench { specular: false, ..Workbench::default() },
        Workbench { shadow: true, ..Workbench::default() },
        Workbench { cavity: true, cavity_type: CavityType::Both, ..Workbench::default() },
        Workbench { outline: true, ..Workbench::default() },
        Workbench { xray: true, ..Workbench::default() },
    ];
    for wb in variants {
        let st = ViewportSettings { workbench: wb, ..base };
        let a = vs.draw(&view, &st);
        assert_ne!(a, plain, "{wb:?} changes the image");
        assert_eq!(vs.draw(&view, &ViewportSettings { threads: 5, ..st }), a, "{wb:?}");
    }
    // The texture shows both checker colours on the cube.
    let tex = vs.draw(&view, &ViewportSettings { workbench: Workbench { color: ColorType::Texture, lighting: Lighting::Flat, ..Workbench::default() }, ..base });
    let yellow = tex.chunks_exact(4).filter(|p| p[0] > 200 && p[1] > 200 && p[2] < 60).count();
    let blue = tex.chunks_exact(4).filter(|p| p[0] < 60 && p[1] < 60 && p[2] > 200).count();
    assert!(yellow > 30 && blue > 30, "{yellow} {blue}");
    // The shadow darkens part of the floor under the monkey and cube.
    let flat = Workbench { lighting: Lighting::Flat, ..Workbench::default() };
    let lit = vs.draw(&view, &ViewportSettings { workbench: flat, grid: false, ..base });
    let shadowed = vs.draw(&view, &ViewportSettings { workbench: Workbench { shadow: true, ..flat }, grid: false, ..base });
    let darker = lit.chunks_exact(4).zip(shadowed.chunks_exact(4)).filter(|(a, b)| (b[0] as i32) < a[0] as i32 - 30).count();
    assert!(darker > 200, "{darker}");
    // X-ray: the floor is visible through the monkey (its pixels change).
    let xr = vs.draw(&view, &ViewportSettings { workbench: Workbench { xray: true, ..flat }, grid: false, ..base });
    assert!(image_diff(&xr, &lit).0 > 1.0);
}

/// Culling objects outside the frustum changes nothing in the picture, and
/// sculpt patches keep matching a rebuild.
#[test]
fn frustum_culling_and_patches() {
    let mut s = monkey_scene();
    // Far behind the camera, outside the view.
    let far = s.add_mesh_object("Far", primitives::ico_sphere(2, 1.0));
    s.objects[far].transform.location = Vec3::new(0.0, -400.0, 0.0);
    let view = monkey_view();
    let st = ViewportSettings { width: 160, height: 90, threads: 3, ..ViewportSettings::default() };
    let with = ViewportScene::new(&s).draw(&view, &st);
    s.remove_object(far);
    assert_eq!(ViewportScene::new(&s).draw(&view, &st), with);

    // A patch moving the cube into view of a culled region still draws it.
    let cube = s.find_object("Cube").unwrap();
    let mi = match s.objects[cube].data {
        ObjectData::Mesh(mi) => mi,
        _ => unreachable!(),
    };
    let mut vs = ViewportScene::new(&s);
    let _ = vs.draw(&view, &st);
    for p in &mut s.meshes[mi].positions {
        *p = *p * 1.6 + Vec3::new(0.0, 0.0, 0.5);
    }
    let all: Vec<u32> = (0..s.meshes[mi].positions.len() as u32).collect();
    assert!(vs.patch_object(cube, &s.meshes[mi].positions, &s.meshes[mi].vertex_normals(), &all));
    let (a, b) = (vs.draw(&view, &st), ViewportScene::new(&s).draw(&view, &st));
    let diff: Vec<usize> = (0..a.len() / 4).filter(|&i| a[i * 4..i * 4 + 4] != b[i * 4..i * 4 + 4]).collect();
    assert!(diff.is_empty(), "{} {:?} {:?} {:?}", diff.len(), &diff[..diff.len().min(5)], &a[diff[0] * 4..diff[0] * 4 + 4], &b[diff[0] * 4..diff[0] * 4 + 4]);
}

/// Picking is unchanged: the ray cast agrees with the visibility buffer
/// over the whole frame (both report the nearest object).
#[test]
fn picking_agrees_with_the_image() {
    let s = monkey_scene();
    let view = monkey_view();
    let (w, h) = (96u32, 54u32);
    let red_obj = s.find_object("Suzanne").unwrap();
    let st = ViewportSettings { width: w, height: h, grid: false, overlays: false, background: Some([0.0; 3]), workbench: Workbench { lighting: Lighting::Flat, ..Workbench::default() }, ..ViewportSettings::default() };
    let px = viewport::draw(&s, &view, &st);
    let vs = ViewportScene::new(&s);
    let mut checked = 0;
    for y in 0..h {
        for x in 0..w {
            let p = &px[((y * w + x) * 4) as usize..][..3];
            let picked = vs.pick(&view, w, h, x as f32 + 0.5, y as f32 + 0.5);
            // Flat red is the monkey; flat grey the cube or floor; black nothing.
            if p[0] > 150 && p[1] < 100 {
                checked += 1;
                assert_eq!(picked, Some(red_obj), "{x},{y}");
            } else if p == [0, 0, 0] {
                assert!(picked.is_none_or(|o| !matches!(s.objects[o].data, ObjectData::Mesh(_))), "{x},{y}");
            }
        }
    }
    assert!(checked > 50);
}

/// F12 with the Workbench engine renders the camera's view, anti-aliased.
#[test]
fn workbench_render_engine() {
    let mut s = monkey_scene();
    add_camera(&mut s, Vec3::new(9.0, -9.0, 6.0), Vec3::ZERO, 0.6);
    let img = sex_3d::render::render_workbench(&s, &WorkbenchRender { width: 120, height: 68, samples: 4, ..WorkbenchRender::default() });
    assert_eq!((img.width, img.height), (120, 68));
    let centre = img.pixels[34 * 120 + 60];
    let corner = img.pixels[0];
    assert!(centre != corner, "{centre:?} vs {corner:?}");
    // Edges get intermediate (anti-aliased) values: more distinct colours
    // than a single sample gives.
    let one = sex_3d::render::render_workbench(&s, &WorkbenchRender { width: 120, height: 68, samples: 1, ..WorkbenchRender::default() });
    let distinct = |im: &sex_3d::scene::Image| {
        let mut v: Vec<[u32; 3]> = im.pixels.iter().map(|p| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()]).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    assert!(distinct(&img) > distinct(&one));
    let t = sex_3d::render::render_workbench(&s, &WorkbenchRender { width: 60, height: 34, samples: 2, film_transparent: true, ..WorkbenchRender::default() });
    assert_eq!(t.pixels[0][3], 0.0);
    assert!(t.pixels[17 * 60 + 30][3] > 0.99);
}

// ------------------------------------------------------------------ EEVEE

use sex_3d::nodes::{NodeTree, TreeKind, Value};
use sex_3d::render::{render_eevee, EeveeSettings, RenderSettings};
use sex_3d::scene::{Image, Light, LightKind};

/// A material tree: `build` returns (node, output socket) for the Surface.
fn node_material(build: impl FnOnce(&mut NodeTree) -> (usize, &'static str)) -> Material {
    let mut t = NodeTree::new("Shader Nodetree", TreeKind::Shader);
    let out = t.add("ShaderNodeOutputMaterial", [300.0, 0.0]).unwrap();
    let (n, s) = build(&mut t);
    t.link(n, s, out, "Surface").unwrap();
    Material { node_tree: Some(t), ..Material::default() }
}

fn diffuse(c: f32) -> Material {
    node_material(|t| {
        let d = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        t.nodes[d].set_input("Color", Value::Color([c, c, c, 1.0]));
        (d, "BSDF")
    })
}

fn emission(color: [f32; 4], strength: f32) -> Material {
    node_material(|t| {
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.nodes[e].set_input("Color", Value::Color(color));
        t.nodes[e].set_input("Strength", Value::Float(strength));
        (e, "Emission")
    })
}

/// Emission mixed half and half with a Transparent BSDF.
fn half_transparent(color: [f32; 4]) -> Material {
    node_material(|t| {
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.nodes[e].set_input("Color", Value::Color(color));
        let tr = t.add("ShaderNodeBsdfTransparent", [0.0, 0.0]).unwrap();
        let mix = t.add("ShaderNodeMixShader", [0.0, 0.0]).unwrap();
        t.nodes[mix].set_input("Fac", Value::Float(0.5));
        t.link(e, "Emission", mix, "Shader").unwrap();
        t.link(tr, "BSDF", mix, "Shader_001").unwrap();
        (mix, "Shader")
    })
}

fn add_sun(s: &mut Scene, from: Vec3, power: f32, angle: f32) {
    let mut o = Object::new("Sun", ObjectData::Light(Light { kind: LightKind::Sun, power, radius: angle, ..Default::default() }));
    o.transform = Transform::from_matrix(&Mat4::look_at(from, Vec3::ZERO, Vec3::Z).inverse());
    s.add_object(o);
}

fn eevee(w: u32, h: u32, samples: u32) -> EeveeSettings {
    EeveeSettings { width: w, height: h, samples, ao: false, volumetrics: false, ..EeveeSettings::default() }
}

fn px(img: &Image, x: u32, y: u32) -> [f32; 4] {
    img.pixels[(y * img.width + x) as usize]
}

/// A diffuse sphere on a floor under a sun, black world: EEVEE's direct
/// light and shadow agree with the path tracer's direct lighting.
#[test]
fn eevee_sun_and_shadow_match_the_path_tracer() {
    let mut s = Scene::new();
    s.world.color = Vec3::ZERO;
    s.world.strength = 1.0;
    let m = s.add_material(diffuse(0.8));
    let mut sphere = primitives::uv_sphere(64, 32, 1.0);
    for f in &mut sphere.faces {
        f.smooth = true;
    }
    let o = s.add_mesh_object("Sphere", sphere);
    s.objects[o].materials.push(m);
    s.objects[o].transform.location = Vec3::new(0.0, 0.0, 1.0);
    let p = s.add_mesh_object("Floor", primitives::plane(12.0));
    s.objects[p].materials.push(m);
    add_sun(&mut s, Vec3::new(2.0, -1.0, 3.0), 3.0, 0.002);
    add_camera(&mut s, Vec3::new(0.0, -8.0, 5.0), Vec3::new(0.0, 0.0, 0.5), 0.6);
    let (w, h) = (96, 64);
    let pt = sex_3d::render::render(&s, &RenderSettings { width: w, height: h, samples: 64, max_bounces: 0, denoise: false, ..RenderSettings::default() }).color;
    let ee = render_eevee(&s, &EeveeSettings { soft_shadows: false, ..eevee(w, h, 8) });
    let (mut sum, mut bad) = (0.0f64, 0);
    let (mut shadow_pt, mut shadow_ee) = (0, 0);
    for (a, b) in pt.pixels.iter().zip(&ee.pixels) {
        let d = (0..3).map(|c| (a[c] - b[c]).abs()).fold(0.0f32, f32::max);
        sum += d as f64;
        bad += (d > 0.12 * a[1].max(0.2)) as usize;
        // Floor in the sphere's shadow (lit floor is ~0.8 * 3 * cos / pi).
        shadow_pt += (a[1] < 0.05) as usize;
        shadow_ee += (b[1] < 0.05) as usize;
    }
    let n = pt.pixels.len();
    let mean = sum / n as f64;
    assert!(mean < 0.03, "mean difference {mean}");
    assert!(bad < n / 25, "{bad} of {n} pixels differ");
    assert!(shadow_pt > n / 50, "the path tracer shows a shadow ({shadow_pt})");
    assert!((shadow_ee as f64 - shadow_pt as f64).abs() < 0.2 * shadow_pt as f64, "shadow pixels: eevee {shadow_ee} vs path tracer {shadow_pt}");
}

/// Emission shows at its exact radiance.
#[test]
fn eevee_emission() {
    let mut s = Scene::new();
    s.world.color = Vec3::ZERO;
    let m = s.add_material(emission([1.0, 0.5, 0.25, 1.0], 2.0));
    let o = s.add_mesh_object("Plane", primitives::plane(4.0));
    s.objects[o].materials.push(m);
    add_camera(&mut s, Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 0.5);
    let img = render_eevee(&s, &eevee(32, 32, 4));
    let c = px(&img, 16, 16);
    assert!((c[0] - 2.0).abs() < 1e-3 && (c[1] - 1.0).abs() < 1e-3 && (c[2] - 0.5).abs() < 1e-3 && (c[3] - 1.0).abs() < 1e-4, "{c:?}");
}

/// Blended transparent surfaces composite back to front, whatever the
/// order of the objects.
#[test]
fn eevee_alpha_blend_order() {
    for flip in [false, true] {
        let mut s = Scene::new();
        s.world.color = Vec3::ZERO;
        let red = s.add_material(half_transparent([1.0, 0.0, 0.0, 1.0]));
        let blue = s.add_material(half_transparent([0.0, 0.0, 1.0, 1.0]));
        let add = |s: &mut Scene, mat: usize, z: f32| {
            let o = s.add_mesh_object("Pane", primitives::plane(4.0));
            s.objects[o].materials.push(mat);
            s.objects[o].transform.location = Vec3::new(0.0, 0.0, z);
        };
        if flip {
            add(&mut s, blue, 0.0);
            add(&mut s, red, 1.0);
        } else {
            add(&mut s, red, 1.0);
            add(&mut s, blue, 0.0);
        }
        add_camera(&mut s, Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, 0.4);
        let c = px(&render_eevee(&s, &EeveeSettings { film_transparent: true, ..eevee(16, 16, 2) }), 8, 8);
        // Front red over back blue over black: 0.5 red + 0.25 blue.
        assert!((c[0] - 0.5).abs() < 0.02 && (c[2] - 0.25).abs() < 0.02 && c[1].abs() < 0.01, "flip {flip}: {c:?}");
        assert!((c[3] - 0.75).abs() < 0.02, "coverage {c:?}");
    }
}

/// A plane lit by a sun at several angles has no shadow acne: every floor
/// pixel gets the same light.
#[test]
fn eevee_shadow_maps_are_acne_free() {
    for elevation in [80.0f32, 45.0, 20.0] {
        let mut s = Scene::new();
        s.world.color = Vec3::ZERO;
        let m = s.add_material(diffuse(0.8));
        let o = s.add_mesh_object("Floor", primitives::plane(40.0));
        s.objects[o].materials.push(m);
        let e = elevation.to_radians();
        add_sun(&mut s, Vec3::new(e.cos() * 0.6, e.cos() * 0.8, e.sin()) * 10.0, 3.0, 0.02);
        add_camera(&mut s, Vec3::new(0.0, -6.0, 3.0), Vec3::ZERO, 0.8);
        let img = render_eevee(&s, &EeveeSettings { film_transparent: true, ..eevee(64, 48, 4) });
        let expect = 0.8 * 3.0 * e.sin() / core::f32::consts::PI;
        let mut floor = 0;
        for (i, p) in img.pixels.iter().enumerate() {
            if p[3] > 0.999 {
                floor += 1;
                assert!((p[1] - expect).abs() < 0.03 * expect, "elevation {elevation}: pixel {i} = {} (want {expect})", p[1]);
            }
        }
        assert!(floor > 1000, "{floor}");
    }
}

/// A checker texture node reaches the deferred shading: two albedos.
#[test]
fn eevee_checker_texture_is_visible() {
    let mut s = Scene::new();
    s.world.color = Vec3::ONE;
    s.world.strength = 1.0;
    let m = s.add_material(node_material(|t| {
        let d = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        let c = t.add("ShaderNodeTexChecker", [0.0, 0.0]).unwrap();
        t.nodes[c].set_input("Color1", Value::Color([0.8, 0.8, 0.8, 1.0]));
        t.nodes[c].set_input("Color2", Value::Color([0.2, 0.2, 0.2, 1.0]));
        t.nodes[c].set_input("Scale", Value::Float(12.0));
        t.link(c, "Color", d, "Color").unwrap();
        (d, "BSDF")
    }));
    let o = s.add_mesh_object("Plane", primitives::plane(4.0));
    s.objects[o].materials.push(m);
    add_camera(&mut s, Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 0.5);
    let img = render_eevee(&s, &eevee(48, 48, 4));
    let (mut light, mut dark) = (0, 0);
    for p in &img.pixels {
        if (p[1] - 0.8).abs() < 0.08 {
            light += 1;
        } else if (p[1] - 0.2).abs() < 0.05 {
            dark += 1;
        }
    }
    let n = img.pixels.len();
    assert!(light > n / 4 && dark > n / 4, "light {light} dark {dark} of {n}");
}

/// Material Preview's studio lighting shows the material colour, and the
/// image does not depend on the thread count.
#[test]
fn eevee_lookdev() {
    let mut s = monkey_scene();
    add_camera(&mut s, Vec3::new(9.0, -9.0, 6.0), Vec3::ZERO, 0.6);
    let st = EeveeSettings { lookdev: true, threads: 1, ..eevee(80, 45, 2) };
    let a = render_eevee(&s, &st);
    let b = render_eevee(&s, &EeveeSettings { threads: 3, ..st });
    assert_eq!(a.pixels, b.pixels);
    assert!(sex_3d::render::eevee::mean_luminance(&a) > 0.05);
    // The red monkey is redder than it is green.
    let reddish = a.pixels.iter().filter(|p| p[0] > 2.0 * p[1] + 0.02).count();
    assert!(reddish > 20, "{reddish}");
}
