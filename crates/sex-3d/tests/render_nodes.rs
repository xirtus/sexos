// SPDX-License-Identifier: GPL-3.0-only
//! Rendering with shader node trees: Principled against the legacy material,
//! emission, Mix Shader, textures, transparency and the world tree. Small
//! images and few samples to keep the run short.
use sex_3d::math::{Mat4, Vec3};
use sex_3d::nodes::{NodeTree, Prop, TreeKind, Value};
use sex_3d::primitives;
use sex_3d::render::{self, RenderSettings};
use sex_3d::scene::{Camera, Image, Material, Object, ObjectData, Scene, Transform};

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

fn settings(w: u32, h: u32, spp: u32) -> RenderSettings {
    RenderSettings { width: w, height: h, samples: spp, max_bounces: 8, threads: 0, denoise: false, clamp: 0.0, seed: 0, light_paths: sex_3d::render::LightPaths::unlimited(), ..RenderSettings::default() }
}

/// A material tree: `build` adds nodes and returns (node, output socket)
/// to plug into the Material Output's Surface.
fn material(build: impl FnOnce(&mut NodeTree) -> (usize, &'static str)) -> Material {
    let mut t = NodeTree::new("Shader Nodetree", TreeKind::Shader);
    let out = t.add("ShaderNodeOutputMaterial", [300.0, 0.0]).unwrap();
    let (n, s) = build(&mut t);
    t.link(n, s, out, "Surface").unwrap();
    Material { node_tree: Some(t), ..Material::default() }
}

/// One object filling the view under a uniform white world.
fn sphere_scene(m: Material, world: f32) -> Scene {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(world);
    s.world.strength = 1.0;
    let mi = s.add_material(m);
    let o = s.add_mesh_object("Sphere", primitives::uv_sphere(48, 24, 1.0));
    s.objects[o].materials.push(mi);
    add_camera(&mut s, Vec3::new(0.0, -4.0, 0.0), Vec3::ZERO, 0.4);
    s
}

fn plane_scene(m: Material, world: f32) -> Scene {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(world);
    s.world.strength = 1.0;
    let mi = s.add_material(m);
    let o = s.add_mesh_object("Plane", primitives::plane(4.0));
    s.objects[o].materials.push(mi);
    add_camera(&mut s, Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO, 0.5);
    s
}

#[test]
fn principled_tree_matches_the_legacy_material() {
    let legacy = Material { base_color: [0.7, 0.4, 0.2, 1.0], roughness: 0.5, metallic: 0.0, ior: 1.5, ..Material::default() };
    let nodes = material(|t| {
        let p = t.add("ShaderNodeBsdfPrincipled", [0.0, 0.0]).unwrap();
        t.nodes[p].set_input("Base Color", Value::Color([0.7, 0.4, 0.2, 1.0]));
        t.nodes[p].set_input("Roughness", Value::Float(0.5));
        (p, "BSDF")
    });
    let st = settings(24, 24, 96);
    let a = mean(&render::render(&sphere_scene(legacy, 1.0), &st).color);
    let mut synced = nodes.clone();
    assert!(synced.sync_from_tree());
    assert_eq!(synced.base_color, [0.7, 0.4, 0.2, 1.0]);
    let b = mean(&render::render(&sphere_scene(nodes, 1.0), &st).color);
    for c in 0..3 {
        assert!((a[c] - b[c]).abs() < 0.06 * a[c].max(0.1), "legacy {a:?} nodes {b:?}");
    }
}

#[test]
fn emission_tree_lights_and_is_seen() {
    let m = material(|t| {
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.nodes[e].set_input("Color", Value::Color([1.0, 0.5, 0.25, 1.0]));
        t.nodes[e].set_input("Strength", Value::Float(2.0));
        (e, "Emission")
    });
    let r = render::render(&plane_scene(m, 0.0), &settings(16, 16, 4));
    let c = r.color.pixels[8 * 16 + 8];
    assert!((c[0] - 2.0).abs() < 1e-3 && (c[1] - 1.0).abs() < 1e-3 && (c[2] - 0.5).abs() < 1e-3, "{c:?}");
}

#[test]
fn mix_shader_halves_two_emitters() {
    let m = material(|t| {
        let a = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.nodes[a].set_input("Color", Value::Color([1.0, 0.0, 0.0, 1.0]));
        let b = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.nodes[b].set_input("Color", Value::Color([0.0, 0.0, 1.0, 1.0]));
        let mix = t.add("ShaderNodeMixShader", [0.0, 0.0]).unwrap();
        t.nodes[mix].set_input("Fac", Value::Float(0.5));
        t.link(a, "Emission", mix, "Shader").unwrap();
        t.link(b, "Emission", mix, "Shader_001").unwrap();
        (mix, "Shader")
    });
    let r = render::render(&plane_scene(m, 0.0), &settings(16, 16, 4));
    let c = r.color.pixels[8 * 16 + 8];
    assert!((c[0] - 0.5).abs() < 1e-3 && c[1].abs() < 1e-3 && (c[2] - 0.5).abs() < 1e-3, "{c:?}");
}

#[test]
fn checker_texture_gives_two_albedos() {
    let m = material(|t| {
        let d = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        let c = t.add("ShaderNodeTexChecker", [0.0, 0.0]).unwrap();
        t.nodes[c].set_input("Color1", Value::Color([0.8, 0.8, 0.8, 1.0]));
        t.nodes[c].set_input("Color2", Value::Color([0.2, 0.2, 0.2, 1.0]));
        t.nodes[c].set_input("Scale", Value::Float(12.0));
        t.link(c, "Color", d, "Color").unwrap();
        (d, "BSDF")
    });
    // A flat plane under a white sky sees the whole upper hemisphere: radiance = albedo.
    let r = render::render(&plane_scene(m, 1.0), &settings(32, 32, 32));
    let (mut light, mut dark) = (0, 0);
    for p in &r.color.pixels {
        if (p[1] - 0.8).abs() < 0.12 {
            light += 1;
        } else if (p[1] - 0.2).abs() < 0.08 {
            dark += 1;
        }
    }
    let n = r.color.pixels.len();
    assert!(light > n / 4 && dark > n / 4, "light {light} dark {dark} of {n}");
}

#[test]
fn world_tree_sets_the_background() {
    let mut s = Scene::new();
    let mut t = NodeTree::new("World", TreeKind::Shader);
    let out = t.add("ShaderNodeOutputWorld", [300.0, 0.0]).unwrap();
    let bg = t.add("ShaderNodeBackground", [0.0, 0.0]).unwrap();
    t.nodes[bg].set_input("Color", Value::Color([0.2, 0.4, 0.6, 1.0]));
    t.nodes[bg].set_input("Strength", Value::Float(2.0));
    t.link(bg, "Background", out, "Surface").unwrap();
    s.world.node_tree = Some(t);
    add_camera(&mut s, Vec3::new(0.0, -4.0, 0.0), Vec3::ZERO, 0.5);
    let r = render::render(&s, &settings(8, 8, 2));
    let c = r.color.pixels[4 * 8 + 4];
    assert!((c[0] - 0.4).abs() < 1e-3 && (c[1] - 0.8).abs() < 1e-3 && (c[2] - 1.2).abs() < 1e-3, "{c:?}");
}

#[test]
fn world_tree_lights_the_scene_and_light_path_splits_camera_rays() {
    // Camera rays see black; everything else sees white (Is Camera Ray).
    let mut s = sphere_scene(Material { base_color: [1.0, 1.0, 1.0, 1.0], ior: 1.0, roughness: 1.0, ..Material::default() }, 0.0);
    let mut t = NodeTree::new("World", TreeKind::Shader);
    let out = t.add("ShaderNodeOutputWorld", [300.0, 0.0]).unwrap();
    let bg = t.add("ShaderNodeBackground", [0.0, 0.0]).unwrap();
    let lp = t.add("ShaderNodeLightPath", [0.0, 0.0]).unwrap();
    let inv = t.add("ShaderNodeMath", [0.0, 0.0]).unwrap();
    t.nodes[inv].set_prop("operation", Prop::Enum("SUBTRACT".into()));
    t.nodes[inv].inputs[0].value = Value::Float(1.0);
    t.link(lp, "Is Camera Ray", inv, "Value_001").unwrap();
    t.link(inv, "Value", bg, "Strength").unwrap();
    t.nodes[bg].set_input("Color", Value::Color([1.0, 1.0, 1.0, 1.0]));
    t.link(bg, "Background", out, "Surface").unwrap();
    s.world.node_tree = Some(t);
    let r = render::render(&s, &settings(24, 24, 32));
    assert!(r.color.pixels[0][0].abs() < 1e-4, "corner sees the camera-ray black");
    let c = r.color.pixels[12 * 24 + 12];
    assert!((c[0] - 1.0).abs() < 0.1, "white furnace sphere {c:?}");
}

#[test]
fn transparent_bsdf_shows_what_is_behind() {
    // A transparent plane over an emitter: the emitter shows through, tinted.
    let mut s = Scene::new();
    s.world.color = Vec3::ZERO;
    let glassy = s.add_material(material(|t| {
        let tr = t.add("ShaderNodeBsdfTransparent", [0.0, 0.0]).unwrap();
        t.nodes[tr].set_input("Color", Value::Color([0.5, 1.0, 1.0, 1.0]));
        (tr, "BSDF")
    }));
    let glow = s.add_material(material(|t| {
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        (e, "Emission")
    }));
    let a = s.add_mesh_object("Front", primitives::plane(4.0));
    s.objects[a].materials.push(glassy);
    s.objects[a].transform.location = Vec3::new(0.0, 0.0, 1.0);
    let b = s.add_mesh_object("Back", primitives::plane(8.0));
    s.objects[b].materials.push(glow);
    add_camera(&mut s, Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, 0.4);
    let r = render::render(&s, &settings(16, 16, 4));
    let c = r.color.pixels[8 * 16 + 8];
    assert!((c[0] - 0.5).abs() < 1e-3 && (c[1] - 1.0).abs() < 1e-3, "{c:?}");
}

#[test]
fn node_emitters_are_sampled_as_lights() {
    // An emissive node quad above a diffuse floor lights it through NEE.
    let mut s = Scene::new();
    s.world.color = Vec3::ZERO;
    let floor = s.add_material(Material { base_color: [0.8, 0.8, 0.8, 1.0], ior: 1.0, roughness: 1.0, ..Material::default() });
    let lamp = s.add_material(material(|t| {
        let e = t.add("ShaderNodeEmission", [0.0, 0.0]).unwrap();
        t.nodes[e].set_input("Strength", Value::Float(10.0));
        (e, "Emission")
    }));
    let f = s.add_mesh_object("Floor", primitives::plane(10.0));
    s.objects[f].materials.push(floor);
    let l = s.add_mesh_object("Lamp", primitives::plane(1.0));
    s.objects[l].materials.push(lamp);
    s.objects[l].transform.location = Vec3::new(0.0, 0.0, 2.0);
    add_camera(&mut s, Vec3::new(0.0, -6.0, 3.0), Vec3::ZERO, 0.6);
    let r = render::render(&s, &settings(24, 24, 16));
    let m = mean(&r.color);
    assert!(m[0] > 0.05, "floor is lit: {m:?}");
}

#[test]
fn node_renders_are_deterministic_across_thread_counts() {
    let m = material(|t| {
        let d = t.add("ShaderNodeBsdfDiffuse", [0.0, 0.0]).unwrap();
        let n = t.add("ShaderNodeTexNoise", [0.0, 0.0]).unwrap();
        t.nodes[n].set_input("Scale", Value::Float(4.0));
        t.link(n, "Color", d, "Color").unwrap();
        (d, "BSDF")
    });
    let s = sphere_scene(m, 1.0);
    let mut st = settings(16, 16, 4);
    st.threads = 1;
    let a = render::render(&s, &st);
    st.threads = 3;
    let b = render::render(&s, &st);
    assert_eq!(a.color.pixels, b.color.pixels);
}
