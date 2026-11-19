// SPDX-License-Identifier: GPL-3.0-only
//! Fluid > Liquid: the FLIP solver, flows, obstacles, the surface mesh, the
//! cache and baking, the renderers; and Dynamic Paint.
use sex_3d::math::{Mat4, Vec3};
use sex_3d::modifiers::Modifier;
use sex_3d::physics::cache::SimFrame;
use sex_3d::physics::liquid::{self, LiquidFrame};
use sex_3d::physics::*;
use sex_3d::scene::{Camera, Light, LightKind, Material, Object, ObjectData, Transform};
use sex_3d::{primitives, Scene};

const KEY: &str = "Domain/fluid0";

/// A closed liquid domain x ∈ [-1, 1], y ∈ [-0.5, 0.5], z ∈ [-0.75, 0.75].
fn domain(s: &mut Scene, res: i32, edit: impl FnOnce(&mut FluidSettings)) -> usize {
    let d = s.add_mesh_object("Domain", primitives::cube(2.0));
    s.objects[d].transform.scale = Vec3::new(1.0, 0.5, 0.75);
    let mut f = FluidSettings::new(FluidType::Domain);
    f.domain_type = DomainType::Liquid;
    f.resolution_max = res;
    f.use_collision_border_front = true;
    f.use_collision_border_back = true;
    f.use_collision_border_left = true;
    f.use_collision_border_right = true;
    f.use_collision_border_top = true;
    f.use_collision_border_bottom = true;
    f.cache_frame_end = 120;
    edit(&mut f);
    s.objects[d].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    d
}

/// A box object `lo..hi` with a fluid modifier.
fn fluid_box(s: &mut Scene, name: &str, lo: Vec3, hi: Vec3, f: FluidSettings) -> usize {
    let o = s.add_mesh_object(name, primitives::cube(1.0));
    s.objects[o].transform.location = (lo + hi) * 0.5;
    s.objects[o].transform.scale = hi - lo;
    s.objects[o].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    o
}

fn flow(behavior: FlowBehavior) -> FluidSettings {
    let mut f = FluidSettings::new(FluidType::Flow);
    f.flow_type = FlowType::Liquid;
    f.flow_behavior = behavior;
    f.surface_distance = 0.0;
    f
}

/// The water column of the dam break: x ∈ [-1, -0.5], the full depth,
/// z ∈ [-0.75, 0.25] (0.5 m³).
fn column(s: &mut Scene) -> usize {
    fluid_box(s, "Column", Vec3::new(-1.0, -0.5, -0.75), Vec3::new(-0.5, 0.5, 0.25), flow(FlowBehavior::Geometry))
}

fn scene() -> Scene {
    let mut s = Scene::new();
    s.objects.clear();
    s
}

fn play(s: &mut Scene, to: i32) {
    for f in s.frame_current..=to {
        s.frame_current = f;
        update(s);
    }
}

fn frame(s: &Scene, f: i32) -> LiquidFrame {
    match &*s.physics.caches.exact(KEY, f).unwrap_or_else(|| panic!("frame {f} cached")) {
        SimFrame::Liquid(l) => l.clone(),
        _ => panic!("not a liquid frame"),
    }
}

#[test]
fn closed_box_keeps_its_volume() {
    let mut s = scene();
    domain(&mut s, 24, |_| {});
    column(&mut s);
    s.frame_current = 1;
    play(&mut s, 60);
    let first = frame(&s, 1);
    let v0 = first.volume();
    assert!((v0 - 0.5).abs() < 0.03, "the column's surface encloses 0.5 m³: {v0}");
    assert!(first.count > 0);
    for f in 2..=60 {
        let fr = frame(&s, f);
        let v = fr.volume();
        if std::env::var("LIQUID_DEBUG").is_ok() {
            eprintln!("frame {f}: volume {v:.4} particles {}", fr.count);
        }
        assert!((v - v0).abs() / v0 < 0.04, "frame {f}: volume {v} vs {v0}");
        assert_eq!(fr.count, first.count, "frame {f}: a closed box keeps every particle");
        assert!(fr.mesh.faces.len() > 50);
    }
}

fn front(f: &LiquidFrame) -> f32 {
    f.mesh.bounds().max.x
}

#[test]
fn dam_break_spreads_and_falls() {
    let mut s = scene();
    domain(&mut s, 24, |_| {});
    column(&mut s);
    s.frame_current = 1;
    play(&mut s, 24);
    let a = frame(&s, 1);
    let b = frame(&s, 24);
    assert!(front(&a) < -0.4, "starts as a column: front {}", front(&a));
    assert!(front(&b) > 0.6, "the water reaches across the floor: front {}", front(&b));
    // The column collapses: the water at the left wall drops (it splashes
    // up the far wall instead).
    let left_top = |f: &LiquidFrame| f.mesh.positions.iter().filter(|p| p.x < -0.7).map(|p| p.z).fold(f32::MIN, f32::max);
    assert!(left_top(&b) < left_top(&a) - 0.4, "the column collapses: {} -> {}", left_top(&a), left_top(&b));
}

#[test]
fn obstacle_blocks_the_flow() {
    let mut s = scene();
    domain(&mut s, 24, |_| {});
    column(&mut s);
    let mut e = FluidSettings::new(FluidType::Effector);
    e.effector_type = EffectorType::Collision;
    fluid_box(&mut s, "Wall", Vec3::new(-0.25, -0.6, -0.8), Vec3::new(0.0, 0.6, 0.8), e);
    s.frame_current = 1;
    play(&mut s, 40);
    for f in [10, 20, 30, 40] {
        let fr = frame(&s, f);
        assert!(front(&fr) < -0.15, "frame {f}: water passed the wall (front {})", front(&fr));
    }
    let v0 = frame(&s, 1).volume();
    let v = frame(&s, 40).volume();
    assert!((v - v0).abs() / v0 < 0.08, "dammed water keeps its volume: {v0} -> {v}");
}

#[test]
fn inflow_adds_and_outflow_removes() {
    // Inflow: a box near the top pouring into an empty domain.
    let mut s = scene();
    domain(&mut s, 20, |_| {});
    let mut f = flow(FlowBehavior::Inflow);
    f.use_initial_velocity = true;
    f.velocity_coord = [0.0, 0.0, -1.0];
    fluid_box(&mut s, "Tap", Vec3::new(-0.2, -0.15, 0.3), Vec3::new(0.2, 0.15, 0.6), f);
    s.frame_current = 1;
    play(&mut s, 30);
    let (c5, c15, c30) = (frame(&s, 5).count, frame(&s, 15).count, frame(&s, 30).count);
    assert!(c5 > 0 && c15 > c5 && c30 > c15, "inflow keeps adding particles: {c5} {c15} {c30}");
    let (v5, v30) = (frame(&s, 5).volume(), frame(&s, 30).volume());
    assert!(v30 > v5 * 1.5 && v30 > 0.05, "inflow adds volume: {v5} -> {v30}");

    // Outflow: a drain on the floor at the far end of the dam break.
    let mut s = scene();
    domain(&mut s, 20, |_| {});
    column(&mut s);
    fluid_box(&mut s, "Drain", Vec3::new(0.4, -0.5, -0.75), Vec3::new(1.0, 0.5, -0.55), flow(FlowBehavior::Outflow));
    s.frame_current = 1;
    play(&mut s, 60);
    let (a, b) = (frame(&s, 1), frame(&s, 60));
    assert!(b.count < a.count * 7 / 10, "outflow removes particles: {} -> {}", a.count, b.count);
    assert!(b.volume() < a.volume() * 0.75, "outflow removes volume: {} -> {}", a.volume(), b.volume());
}

#[test]
fn open_borders_let_the_liquid_out() {
    let mut s = scene();
    domain(&mut s, 16, |f| f.use_collision_border_bottom = false);
    column(&mut s);
    s.frame_current = 1;
    play(&mut s, 30);
    assert!(frame(&s, 30).count < frame(&s, 1).count / 4, "liquid falls out of the open bottom");
}

#[test]
fn bake_and_free_round_trip() {
    let mut s = scene();
    domain(&mut s, 16, |f| {
        f.cache_type = FluidCacheType::All;
        f.cache_frame_end = 12;
    });
    column(&mut s);
    // Cache Type All: nothing simulates on frame changes.
    s.frame_current = 1;
    play(&mut s, 6);
    assert_eq!(s.physics.caches.status(KEY).3, 0, "All only simulates when baked");
    let n = bake(&mut s, &[KEY.to_string()], &mut |_, _| {}).unwrap();
    assert_eq!(n, 1);
    assert_eq!(s.frame_current, 6, "the frame is restored");
    let (_, baked, _, frames) = s.physics.caches.status(KEY);
    assert!(baked && frames == 12, "baked 1..=12: {frames}");
    let baked10 = frame(&s, 10);
    assert!(baked10.mesh.faces.len() > 20);
    // The domain shows the baked liquid.
    s.frame_current = 10;
    let shown = s.evaluated_mesh(0).unwrap();
    assert_eq!(shown.faces.len(), baked10.mesh.faces.len());
    // A baked cache ignores setting changes.
    if let Modifier::Physics(PhysicsModifier::Fluid(f)) = &mut s.objects[0].modifiers[0] {
        f.flip_ratio = 0.5;
    }
    update(&s);
    assert_eq!(s.physics.caches.status(KEY).3, 12);
    if let Modifier::Physics(PhysicsModifier::Fluid(f)) = &mut s.objects[0].modifiers[0] {
        f.flip_ratio = 0.97;
    }
    // Free: gone, and frame changes do not bring it back.
    free_bakes(&s, &[KEY.to_string()]);
    assert_eq!(s.physics.caches.status(KEY).3, 0);
    assert!(!s.physics.caches.is_baked(KEY));
    play(&mut s, 12);
    assert_eq!(s.physics.caches.status(KEY).3, 0);
    // Without a cache the domain shows its own box.
    assert_eq!(s.evaluated_mesh(0).unwrap().positions.len(), 8);
    // Replay gives the same frames as the bake.
    if let Modifier::Physics(PhysicsModifier::Fluid(f)) = &mut s.objects[0].modifiers[0] {
        f.cache_type = FluidCacheType::Replay;
    }
    s.frame_current = 1;
    play(&mut s, 10);
    assert_eq!(frame(&s, 10).mesh.positions, baked10.mesh.positions, "replay and bake agree");
}

fn signature(threads: usize, method: SimulationMethod) -> (Vec<u32>, usize) {
    liquid::set_threads(threads);
    let mut s = scene();
    domain(&mut s, 40, |f| {
        f.simulation_method = method;
        f.use_flip_particles = true;
    });
    column(&mut s);
    let mut f = flow(FlowBehavior::Inflow);
    f.use_initial_velocity = true;
    f.velocity_coord = [1.0, 0.0, 0.0];
    fluid_box(&mut s, "Tap", Vec3::new(0.3, -0.1, 0.3), Vec3::new(0.5, 0.1, 0.5), f);
    s.frame_current = 1;
    play(&mut s, 12);
    let fr = frame(&s, 12);
    liquid::set_threads(0);
    let mut bits: Vec<u32> = fr.mesh.positions.iter().flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]).collect();
    bits.extend(fr.particles.as_ref().unwrap().iter().flat_map(|p| [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]));
    (bits, fr.count)
}

#[test]
fn deterministic_across_thread_counts() {
    let one = signature(1, SimulationMethod::Flip);
    let four = signature(4, SimulationMethod::Flip);
    assert!(one.1 > 10000, "large enough to run in parallel: {}", one.1);
    assert_eq!(one.1, four.1);
    assert!(one.0 == four.0, "1 and 4 threads give identical results");
    let apic1 = signature(1, SimulationMethod::Apic);
    let apic3 = signature(3, SimulationMethod::Apic);
    assert!(apic1 == apic3, "APIC too");
}

#[test]
fn apic_and_viscosity() {
    // APIC: the dam break stays sane.
    let mut s = scene();
    domain(&mut s, 20, |f| f.simulation_method = SimulationMethod::Apic);
    column(&mut s);
    s.frame_current = 1;
    play(&mut s, 30);
    let (a, b) = (frame(&s, 1), frame(&s, 30));
    assert!((b.volume() - a.volume()).abs() / a.volume() < 0.08, "APIC volume {} -> {}", a.volume(), b.volume());
    assert!(front(&b) > 0.3);
    // A very viscous liquid spreads slower than water.
    let run = |visc: bool| {
        let mut s = scene();
        domain(&mut s, 20, |f| {
            f.use_viscosity = visc;
            f.viscosity_value = 2.0;
        });
        column(&mut s);
        s.frame_current = 1;
        play(&mut s, 10);
        front(&frame(&s, 10))
    };
    let (water, honey) = (run(false), run(true));
    assert!(honey < water - 0.2, "viscosity slows the front: water {water}, viscous {honey}");
}

#[test]
fn surface_tension_rounds_a_drop() {
    // A cube of liquid in zero gravity: surface tension pulls it round.
    let run = |sigma: f32| {
        let mut s = scene();
        s.physics.gravity.use_gravity = false;
        domain(&mut s, 24, |f| {
            f.gravity = [0.0; 3];
            f.use_diffusion = sigma > 0.0;
            f.surface_tension = sigma;
        });
        fluid_box(&mut s, "Drop", Vec3::new(-0.3, -0.3, -0.3), Vec3::new(0.3, 0.3, 0.3), flow(FlowBehavior::Geometry));
        s.frame_current = 1;
        play(&mut s, 20);
        let m = &frame(&s, 20).mesh;
        // Corners of the cube: how far the surface reaches along a diagonal.
        m.positions.iter().map(|p| (p.x.abs() + p.y.abs() + p.z.abs()) / 3.0).fold(0.0f32, f32::max)
    };
    let (still, tense) = (run(0.0), run(0.05));
    assert!(tense < still - 0.01, "surface tension rounds the corners: {still} vs {tense}");
}

#[test]
fn upres_and_preview_meshes() {
    let mesh = |edit: fn(&mut FluidSettings)| {
        let mut s = scene();
        domain(&mut s, 16, edit);
        column(&mut s);
        s.frame_current = 1;
        update(&s);
        frame(&s, 1)
    };
    let fine = mesh(|f| f.mesh_scale = 3);
    let preview = mesh(|f| f.mesh_generator = MeshGenerator::Union);
    let off = mesh(|f| f.use_mesh = false);
    assert!(fine.mesh.faces.len() > preview.mesh.faces.len() * 3, "upres: {} vs {}", fine.mesh.faces.len(), preview.mesh.faces.len());
    assert!((fine.volume() - 0.5).abs() < 0.03 && (preview.volume() - 0.5).abs() < 0.05, "{} {}", fine.volume(), preview.volume());
    assert!(off.mesh.faces.is_empty() && off.count > 0);
    assert!(fine.mesh.faces.iter().all(|f| f.smooth && f.verts.len() == 3));
}

#[test]
fn liquid_settings_persist_in_polyops() {
    let mut s = scene();
    domain(&mut s, 33, |f| {
        f.flip_ratio = 0.8;
        f.simulation_method = SimulationMethod::Apic;
        f.use_collision_border_top = false;
        f.mesh_scale = 3;
        f.use_diffusion = true;
        f.surface_tension = 0.2;
        f.use_viscosity = true;
        f.viscosity_value = 0.3;
        f.cache_type = FluidCacheType::Modular;
        f.delete_in_obstacle = true;
        f.fluid_group = "Flows".into();
    });
    let mut fl = flow(FlowBehavior::Inflow);
    fl.use_inflow = false;
    fl.subframes = 2;
    fluid_box(&mut s, "Tap", Vec3::splat(-0.1), Vec3::splat(0.1), fl);
    let dp = DynamicPaintSettings { use_canvas: true, surface_type: DpSurfaceType::Wave, wave_speed: 2.0, ..Default::default() };
    s.objects[1].modifiers.push(Modifier::Physics(PhysicsModifier::DynamicPaint(Box::new(dp))));
    let text = sex_3d::script::scene_to_polyops(&s);
    let mut t = Scene::new();
    t.objects.clear();
    sex_3d::script::run(&mut t, &text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    for o in &s.objects {
        let u = t.objects.iter().find(|x| x.name == o.name).unwrap();
        assert_eq!(u.modifiers, o.modifiers, "{}\n{text}", o.name);
    }
}

// ---------------------------------------------------------------- renders

fn render_scene(with_liquid: bool) -> Scene {
    let mut s = scene();
    let d = domain(&mut s, 20, |_| {});
    column(&mut s);
    let red = s.add_material(Material { base_color: [0.9, 0.1, 0.05, 1.0], ..Default::default() });
    s.objects[d].materials.push(red);
    s.objects.iter_mut().filter(|o| o.name == "Column").for_each(|o| o.visible = false);
    if !with_liquid {
        s.objects[d].visible = false;
    }
    s.world.color = Vec3::splat(0.6);
    s.world.strength = 1.0;
    let mut sun = Object::new("Sun", ObjectData::Light(Light { kind: LightKind::Sun, power: 3.0, ..Default::default() }));
    sun.transform = Transform::from_matrix(&Mat4::look_at(Vec3::new(1.0, -2.0, 3.0), Vec3::ZERO, Vec3::Z).inverse());
    s.add_object(sun);
    let mut cam = Object::new("Camera", ObjectData::Camera(Camera::default()));
    cam.transform = Transform::from_matrix(&Mat4::look_at(Vec3::new(0.0, -5.0, 0.0), Vec3::ZERO, Vec3::Z).inverse());
    let c = s.add_object(cam);
    s.active_camera = Some(c);
    s.frame_current = 1;
    update(&s);
    s
}

/// Pixels that differ between the renders, in the left and right thirds.
fn coverage(a: &[[f32; 4]], b: &[[f32; 4]], w: usize) -> (usize, usize) {
    let (mut l, mut r) = (0, 0);
    for (i, (p, q)) in a.iter().zip(b).enumerate() {
        let d = (0..3).map(|c| (p[c] - q[c]).abs()).fold(0.0f32, f32::max);
        if d > 0.05 {
            let x = i % w;
            if x < w / 3 {
                l += 1;
            } else if x >= 2 * w / 3 {
                r += 1;
            }
        }
    }
    (l, r)
}

#[test]
fn every_renderer_shows_the_liquid() {
    use sex_3d::render::workbench::{render_workbench, WorkbenchRender};
    use sex_3d::render::{render, render_eevee, EeveeSettings, RenderSettings};
    let (with, without) = (render_scene(true), render_scene(false));
    let (w, h) = (64u32, 48u32);
    let pt = |s: &Scene| render(s, &RenderSettings { width: w, height: h, samples: 4, max_bounces: 1, denoise: false, ..RenderSettings::default() }).color.pixels;
    let ee = |s: &Scene| render_eevee(s, &EeveeSettings { width: w, height: h, samples: 1, ao: false, volumetrics: false, ..EeveeSettings::default() }).pixels;
    let wb = |s: &Scene| render_workbench(s, &WorkbenchRender { width: w, height: h, ..WorkbenchRender::default() }).pixels;
    for (name, a, b) in [("Cycles", pt(&with), pt(&without)), ("EEVEE", ee(&with), ee(&without)), ("Workbench", wb(&with), wb(&without))] {
        let (l, r) = coverage(&a, &b, w as usize);
        assert!(l > 40, "{name}: the column shows on the left ({l} px)");
        assert!(r < l / 4, "{name}: nothing on the right ({r} px vs {l})");
    }
}

// ---------------------------------------------------------- dynamic paint

#[test]
fn dynamic_paint_paints_dents_and_weighs() {
    let mk = |t: DpSurfaceType| {
        let mut s = scene();
        let c = s.add_mesh_object("Canvas", primitives::grid(20, 20, 2.0));
        let dp = DynamicPaintSettings { use_canvas: true, surface_type: t, frame_end: 30, ..Default::default() };
        s.objects[c].modifiers.push(Modifier::Physics(PhysicsModifier::DynamicPaint(Box::new(dp))));
        let b = s.add_mesh_object("Brush", primitives::uv_sphere(16, 8, 0.3));
        s.objects[b].transform.location = Vec3::new(-0.5, 0.0, 0.1);
        let br = DynamicPaintSettings { ui_type: DpUiType::Brush, use_brush: true, paint_color: [1.0, 0.0, 0.0], ..Default::default() };
        s.objects[b].modifiers.push(Modifier::Physics(PhysicsModifier::DynamicPaint(Box::new(br))));
        s.frame_current = 1;
        update(&s);
        // The brush moves to the other side.
        s.objects[b].transform.location = Vec3::new(0.5, 0.0, 0.1);
        s.frame_current = 2;
        update(&s);
        (s, c)
    };
    let (s, c) = mk(DpSurfaceType::Paint);
    let m = s.evaluated_mesh(c).unwrap();
    let paint = m.color_attributes.iter().find(|a| a.name == "dp_paintmap").expect("paintmap");
    let wet = m.color_attributes.iter().find(|a| a.name == "dp_wetmap").expect("wetmap");
    let painted = |x: f32| m.positions.iter().zip(&paint.data).filter(|(p, c)| (p.x - x).abs() < 0.15 && p.y.abs() < 0.15 && c[3] > 0.5 && c[0] > 0.9).count();
    assert!(painted(-0.5) > 0 && painted(0.5) > 0, "both brush positions painted");
    assert!(m.positions.iter().zip(&paint.data).filter(|(p, _)| p.x.abs() > 0.9).all(|(_, c)| c[3] == 0.0), "far vertices untouched");
    assert!(wet.data.iter().any(|c| c[0] > 0.5));
    let (s, c) = mk(DpSurfaceType::Displace);
    let m = s.evaluated_mesh(c).unwrap();
    assert!(m.positions.iter().any(|p| p.z < -0.05), "displace dents the canvas");
    let (s, c) = mk(DpSurfaceType::Weight);
    let m = s.evaluated_mesh(c).unwrap();
    let g = m.vertex_groups.iter().position(|n| n == "dp_weight").expect("dp_weight group") as u32;
    assert!(m.weights.iter().any(|w| w.iter().any(|e| e.0 == g && e.1 > 0.9)));
    let (s, c) = mk(DpSurfaceType::Wave);
    let mut s = s;
    for f in 3..8 {
        s.frame_current = f;
        update(&s);
    }
    let m = s.evaluated_mesh(c).unwrap();
    assert!(m.positions.iter().any(|p| p.z.abs() > 1e-3), "waves move the surface");
}
