// SPDX-License-Identifier: GPL-3.0-only
//! Phase 8: rigid bodies, cloth, soft bodies, particles, force fields,
//! smoke and the point cache.
use sex_3d::math::{Quat, Vec3};
use sex_3d::modifiers::Modifier;
use sex_3d::physics::cache::SimFrame;
use sex_3d::physics::field::{Effector, Effectors};
use sex_3d::physics::particles::{self, Alive};
use sex_3d::physics::rigid::{self, RigidFrame};
use sex_3d::physics::*;
use sex_3d::scene::{Object, ObjectData};
use sex_3d::{primitives, Scene};
use std::time::Instant;

/// Play the scene from its current frame to `to`, stepping the simulations.
fn play(scene: &mut Scene, to: i32) {
    for f in scene.frame_current..=to {
        scene.frame_current = f;
        update(scene);
    }
}

fn rigid_frame(scene: &Scene, f: i32) -> RigidFrame {
    match &*scene.physics.caches.exact(rigid::CACHE_KEY, f).expect("cached frame") {
        SimFrame::Rigid(r) => r.clone(),
        _ => panic!("not a rigid frame"),
    }
}

fn rb_scene() -> Scene {
    let mut s = Scene::new();
    s.physics.rigidbody_world = Some(RigidBodyWorld::default());
    s
}

fn add_body(s: &mut Scene, name: &str, mesh: sex_3d::Mesh, pos: Vec3, kind: RigidBodyType, shape: CollisionShape) -> usize {
    let i = s.add_mesh_object(name, mesh);
    s.objects[i].transform.location = pos;
    s.objects[i].physics.rigid_body = Some(RigidBody { kind, collision_shape: shape, ..Default::default() });
    i
}

fn ground(s: &mut Scene) -> usize {
    add_body(s, "Ground", primitives::plane(20.0), Vec3::ZERO, RigidBodyType::Passive, CollisionShape::Box)
}

fn z_of(s: &Scene, i: usize) -> f32 {
    s.world_matrix(i).transform_point(Vec3::ZERO).z
}

#[test]
fn box_drop_rests_on_passive_plane_and_sleeps() {
    let mut s = rb_scene();
    ground(&mut s);
    let c = add_body(&mut s, "Cube", primitives::cube(1.0), Vec3::new(0.0, 0.0, 2.0), RigidBodyType::Active, CollisionShape::Box);
    s.objects[c].physics.rigid_body.as_mut().unwrap().use_deactivation = true;
    play(&mut s, 120);
    let z = z_of(&s, c);
    assert!((z - 0.5).abs() < 0.02, "rest height {z}");
    let p = s.world_matrix(c).transform_point(Vec3::ZERO);
    assert!(p.x.abs() < 0.01 && p.y.abs() < 0.01, "drifted {p:?}");
    let st = rigid_frame(&s, 120).bodies["Cube"];
    assert!(st.sleeping, "body should sleep: {st:?}");
    // Falling: frame 10 is below the start and still above the ground.
    s.frame_current = 10;
    let z10 = z_of(&s, c);
    assert!(z10 < 2.0 && z10 > 0.5, "{z10}");
    // Before the start frame the object is where it was put.
    s.frame_current = 0;
    assert_eq!(z_of(&s, c), 2.0);
}

#[test]
fn five_box_stack_stays_stable() {
    let mut s = rb_scene();
    ground(&mut s);
    let boxes: Vec<usize> = (0..5).map(|k| add_body(&mut s, &format!("Box{k}"), primitives::cube(1.0), Vec3::new(0.0, 0.0, 0.5 + k as f32 * 1.0), RigidBodyType::Active, CollisionShape::Box)).collect();
    play(&mut s, 120);
    for (k, &b) in boxes.iter().enumerate() {
        let p = s.world_matrix(b).transform_point(Vec3::ZERO);
        assert!((p.z - (0.5 + k as f32)).abs() < 0.05, "box {k} at {p:?}");
        assert!(p.x.abs() < 0.05 && p.y.abs() < 0.05, "box {k} slid to {p:?}");
        let up = s.world_matrix(b).transform_vector(Vec3::Z).normalize();
        assert!(up.z > 0.995, "box {k} tipped: {up:?}");
    }
}

#[test]
fn hinge_keeps_its_distance() {
    let mut s = rb_scene();
    let anchor = add_body(&mut s, "Anchor", primitives::cube(0.2), Vec3::new(0.0, 0.0, 3.0), RigidBodyType::Passive, CollisionShape::Box);
    let arm = add_body(&mut s, "Arm", primitives::cube(0.4), Vec3::new(1.5, 0.0, 3.0), RigidBodyType::Active, CollisionShape::Box);
    let mut e = Object::new("Constraint", ObjectData::Empty);
    e.transform.location = Vec3::new(0.0, 0.0, 3.0);
    // Hinges turn about the constraint's Z: lay it along Y.
    e.transform.rotation = Quat::from_axis_angle(Vec3::X, std::f32::consts::FRAC_PI_2);
    e.physics.rigid_body_constraint = Some(RigidBodyConstraint { kind: ConstraintType::Hinge, object1: "Anchor".into(), object2: "Arm".into(), ..Default::default() });
    s.add_object(e);
    let pivot = Vec3::new(0.0, 0.0, 3.0);
    let mut lowest = f32::MAX;
    for f in 1..=72 {
        s.frame_current = f;
        update(&s);
        let p = s.world_matrix(arm).transform_point(Vec3::ZERO);
        let d = (p - pivot).length();
        assert!((d - 1.5).abs() < 0.05, "frame {f}: distance {d}");
        assert!(p.y.abs() < 0.02, "frame {f}: left the hinge plane {p:?}");
        lowest = lowest.min(p.z);
    }
    assert!(lowest < 2.0, "the arm should swing down: {lowest}");
    let _ = anchor;
}

/// Highest point of the sphere's bottom after its first bounce.
fn bounce_height(e: f32) -> f32 {
    let mut s = rb_scene();
    ground(&mut s);
    let b = add_body(&mut s, "Ball", primitives::uv_sphere(24, 12, 0.5), Vec3::new(0.0, 0.0, 2.5), RigidBodyType::Active, CollisionShape::Sphere);
    s.objects[b].physics.rigid_body.as_mut().unwrap().restitution = e;
    s.objects[0].physics.rigid_body.as_mut().unwrap().restitution = 1.0;
    for rb in s.objects.iter_mut().filter_map(|o| o.physics.rigid_body.as_mut()) {
        rb.linear_damping = 0.0;
        rb.angular_damping = 0.0;
    }
    let mut touched = false;
    let mut best = 0.0f32;
    for f in 1..=60 {
        s.frame_current = f;
        update(&s);
        let z = z_of(&s, b) - 0.5;
        if z < 0.1 {
            touched = true;
        } else if touched {
            best = best.max(z);
        }
    }
    assert!(touched, "the ball never landed");
    best
}

#[test]
fn restitution_controls_bounce_energy() {
    let drop = 2.0;
    let none = bounce_height(0.0);
    let half = bounce_height(0.5);
    let full = bounce_height(1.0);
    assert!(none < 0.1, "no restitution should not bounce: {none}");
    // Height scales with e² (energy), allowing for the step size.
    assert!((half / drop - 0.25).abs() < 0.05, "e=0.5 bounce {half}");
    assert!(full <= drop * 1.02, "energy grew: {full}");
    assert!(full > drop * 0.8, "e=1 lost too much: {full}");
}

fn cloth_grid(s: &mut Scene, n: u32, size: f32, z: f32) -> usize {
    let i = s.add_mesh_object("Cloth", primitives::grid(n, n, size));
    s.objects[i].transform.location = Vec3::new(0.0, 0.0, z);
    i
}

#[test]
fn pinned_cloth_corners_hold() {
    let mut s = Scene::new();
    let c = cloth_grid(&mut s, 12, 2.0, 2.0);
    let ObjectData::Mesh(mi) = s.objects[c].data else { unreachable!() };
    let m = &mut s.meshes[mi];
    m.vertex_groups = vec!["Pin".into()];
    let (min, max) = (m.bounds().min, m.bounds().max);
    let corner = |p: Vec3| ((p.x - min.x).abs() < 1e-4 || (p.x - max.x).abs() < 1e-4) && ((p.y - min.y).abs() < 1e-4 || (p.y - max.y).abs() < 1e-4);
    let pins: Vec<usize> = (0..m.positions.len()).filter(|&v| corner(m.positions[v])).collect();
    assert_eq!(pins.len(), 4);
    m.weights = (0..m.positions.len()).map(|v| if pins.contains(&v) { vec![(0, 1.0)] } else { Vec::new() }).collect();
    let rest: Vec<Vec3> = pins.iter().map(|&v| m.positions[v] + Vec3::new(0.0, 0.0, 2.0)).collect();
    let mut cl = ClothSettings::default();
    cl.vertex_group_mass = "Pin".into();
    s.objects[c].modifiers.push(Modifier::Physics(PhysicsModifier::Cloth(Box::new(cl))));
    play(&mut s, 40);
    let ev = s.evaluated_mesh(c).unwrap();
    let w = s.world_matrix(c);
    for (k, &v) in pins.iter().enumerate() {
        let p = w.transform_point(ev.positions[v]);
        assert!(p.approx_eq(rest[k], 1e-3), "pin {v} moved to {p:?}");
    }
    let lowest = ev.positions.iter().map(|&p| w.transform_point(p).z).fold(f32::MAX, f32::min);
    assert!(lowest < 1.8, "the cloth should sag: {lowest}");
    assert!(lowest > -1.0, "the cloth fell away: {lowest}");
}

#[test]
fn cloth_drapes_over_sphere() {
    let mut s = Scene::new();
    let sph = s.add_mesh_object("Sphere", primitives::uv_sphere(32, 16, 1.0));
    s.objects[sph].modifiers.push(Modifier::Physics(PhysicsModifier::Collision(CollisionSettings::default())));
    let c = cloth_grid(&mut s, 24, 3.0, 1.5);
    s.objects[c].modifiers.push(Modifier::Physics(PhysicsModifier::Cloth(Box::default())));
    play(&mut s, 60);
    let ev = s.evaluated_mesh(c).unwrap();
    let w = s.world_matrix(c);
    let mut min_r = f32::MAX;
    let mut top = f32::MIN;
    for &p in &ev.positions {
        let q = w.transform_point(p);
        min_r = min_r.min(q.length());
        if q.x.abs() < 0.2 && q.y.abs() < 0.2 {
            top = top.max(q.z);
        }
    }
    assert!(min_r > 0.95, "cloth passed into the sphere: {min_r}");
    assert!(top < 1.4 && top > 0.95, "the middle should rest on top: {top}");
    let lowest = ev.positions.iter().map(|&p| w.transform_point(p).z).fold(f32::MAX, f32::min);
    assert!(lowest < 0.6, "the edges should hang down: {lowest}");
}

#[test]
fn soft_body_falls_and_keeps_shape() {
    let mut s = Scene::new();
    ground(&mut s);
    s.objects[0].physics.rigid_body = None;
    s.objects[0].modifiers.push(Modifier::Physics(PhysicsModifier::Collision(CollisionSettings::default())));
    let b = s.add_mesh_object("Soft", primitives::cube(1.0));
    s.objects[b].transform.location = Vec3::new(0.0, 0.0, 2.0);
    let sb = SoftBodySettings { use_goal: false, ..Default::default() };
    s.objects[b].modifiers.push(Modifier::Physics(PhysicsModifier::SoftBody(Box::new(sb))));
    play(&mut s, 60);
    let ev = s.evaluated_mesh(b).unwrap();
    let w = s.world_matrix(b);
    let pts: Vec<Vec3> = ev.positions.iter().map(|&p| w.transform_point(p)).collect();
    let low = pts.iter().map(|p| p.z).fold(f32::MAX, f32::min);
    let high = pts.iter().map(|p| p.z).fold(f32::MIN, f32::max);
    assert!(low > -0.05 && low < 0.3, "soft body should land on the ground: {low}");
    assert!(high - low > 0.5, "soft body collapsed: {}", high - low);
}

fn emitter_scene(seed: i32) -> (Scene, usize) {
    let mut s = Scene::new();
    let e = s.add_mesh_object("Emitter", primitives::plane(2.0));
    let mut sys = ParticleSystem::default();
    sys.seed = seed;
    sys.settings.count = 200;
    sys.settings.frame_start = 1.0;
    sys.settings.frame_end = 51.0;
    sys.settings.lifetime = 20.0;
    sys.settings.lifetime_random = 0.0;
    s.objects[e].modifiers.push(Modifier::Physics(PhysicsModifier::ParticleSystem(Box::new(sys))));
    (s, e)
}

fn particle_frame(s: &Scene, f: i32) -> particles::ParticlesFrame {
    match &*s.physics.caches.exact(&particles::key("Emitter", 0), f).expect("cached particles") {
        SimFrame::Particles(p) => p.clone(),
        _ => panic!(),
    }
}

#[test]
fn particles_are_deterministic_with_timing_and_lifetime() {
    let (mut a, _) = emitter_scene(3);
    play(&mut a, 40);
    let (mut b, _) = emitter_scene(3);
    play(&mut b, 40);
    let (mut c, _) = emitter_scene(4);
    play(&mut c, 40);
    let (fa, fb, fc) = (particle_frame(&a, 40), particle_frame(&b, 40), particle_frame(&c, 40));
    assert_eq!(fa.particles.len(), 200);
    for (x, y) in fa.particles.iter().zip(&fb.particles) {
        assert_eq!(x.pos, y.pos);
        assert_eq!(x.state, y.state);
    }
    assert!(fa.particles.iter().zip(&fc.particles).any(|(x, y)| x.pos != y.pos), "another seed should differ");
    for (i, p) in fa.particles.iter().enumerate() {
        // Births spread evenly over the emission frames; lifetime exact.
        let birth = 1.0 + 50.0 * i as f32 / 200.0;
        assert!((p.birth - birth).abs() < 1e-3, "particle {i} born {} not {birth}", p.birth);
        assert!((p.die - p.birth - 20.0).abs() < 1e-3);
        let expect = if p.birth > 40.0 {
            Alive::Unborn
        } else if p.die <= 40.0 {
            Alive::Dead
        } else {
            Alive::Alive
        };
        assert_eq!(p.state, expect, "particle {i}");
    }
    // Born before frame 40 and not yet 20 frames old: births in (20, 40].
    let alive = fa.particles.iter().filter(|p| p.state == Alive::Alive).count();
    assert!((79..=81).contains(&alive), "{alive}");
    // Particles fall with gravity after leaving the emitter upwards.
    let p0 = &fa.particles[150];
    assert!(p0.vel.z < 1.0, "{:?}", p0.vel);
}

#[test]
fn particle_lifetime_random_and_hair() {
    let (mut s, e) = emitter_scene(9);
    if let Modifier::Physics(PhysicsModifier::ParticleSystem(p)) = &mut s.objects[e].modifiers[0] {
        p.settings.lifetime_random = 0.5;
    }
    play(&mut s, 2);
    let f = particle_frame(&s, 2);
    let lives: Vec<f32> = f.particles.iter().map(|p| p.die - p.birth).collect();
    assert!(lives.iter().all(|&l| (10.0 - 1e-3..=20.0 + 1e-3).contains(&l)));
    assert!(lives.iter().any(|&l| l < 15.0) && lives.iter().any(|&l| l > 15.0));

    // Hair: static strands from the faces, with children.
    let mut h = Scene::new();
    let e = h.add_mesh_object("Emitter", primitives::plane(2.0));
    let mut sys = ParticleSystem::default();
    sys.settings = ParticleSettings::hair();
    sys.settings.count = 50;
    sys.settings.child_type = ChildType::Simple;
    sys.settings.child_nbr = 4;
    sys.settings.rendered_child_count = 4;
    h.objects[e].modifiers.push(Modifier::Physics(PhysicsModifier::ParticleSystem(Box::new(sys.clone()))));
    let v = particles::view(&h, e, 0, &sys, true);
    assert_eq!(v.strands.len(), 50 * 5);
    let len = sys.settings.hair_length;
    for st in &v.strands[..50] {
        let l: f32 = st.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        assert!((l - len).abs() < 1e-3 * len.max(1.0) + 1e-3, "{l}");
        assert!(st[0].z.abs() < 1e-4);
    }
    let (pos, _, tris) = particles::hair_tubes(&v.strands, &sys.settings);
    assert!(!tris.is_empty() && tris.iter().all(|t| t.iter().all(|&k| (k as usize) < pos.len())));
}

fn field(kind: FieldType, f: impl FnOnce(&mut FieldSettings)) -> Effector {
    let mut s = FieldSettings::new(kind);
    s.shape = FieldShape::Point;
    f(&mut s);
    Effector { object: 0, settings: s, pos: Vec3::ZERO, nor: Vec3::Z }
}

#[test]
fn force_field_direction_and_falloff() {
    let e = field(FieldType::Force, |s| {
        s.strength = 2.0;
        s.falloff_power = 2.0;
    });
    let f1 = e.force(Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, 0.0);
    let f3 = e.force(Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, 0.0);
    assert!(f1.x > 0.0 && f1.y.abs() < 1e-6 && f1.z.abs() < 1e-6, "positive force pushes away: {f1:?}");
    // Blender's falloff: (1 + d)^-power.
    assert!((f1.length() / f3.length() - 4.0).abs() < 1e-3, "{f1:?} {f3:?}");
    assert!((f1.length() - 2.0 / 4.0).abs() < 1e-4);
    // Negative strength attracts.
    let a = field(FieldType::Force, |s| s.strength = -1.0).force(Vec3::new(0.0, 2.0, 0.0), Vec3::ZERO, 0.0);
    assert!(a.y < 0.0);
    // Max distance cuts off.
    let m = field(FieldType::Force, |s| {
        s.use_max_distance = true;
        s.distance_max = 2.0;
    });
    assert_eq!(m.force(Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO, 0.0), Vec3::ZERO);
    assert!(m.force(Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, 0.0).x > 0.0);
    // Wind blows along the field's Z regardless of position.
    let w = field(FieldType::Wind, |s| s.strength = 3.0).force(Vec3::new(5.0, -2.0, 1.0), Vec3::ZERO, 0.0);
    assert!(w.approx_eq(Vec3::new(0.0, 0.0, 3.0), 1e-5), "{w:?}");
    // Vortex turns around Z, counter-clockwise for positive strength.
    let v = field(FieldType::Vortex, |s| s.strength = 1.0).force(Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, 0.0);
    assert!(v.y > 0.9 && v.x.abs() < 1e-5, "{v:?}");
    // Drag opposes motion.
    let d = field(FieldType::Drag, |_| {}).force(Vec3::ONE, Vec3::new(2.0, 0.0, 0.0), 0.0);
    assert!(d.x < 0.0 && d.y.abs() < 1e-6);
    // Harmonic pulls back towards the field.
    let h = field(FieldType::Harmonic, |s| s.strength = 1.0).force(Vec3::new(0.0, 0.0, 2.0), Vec3::ZERO, 0.0);
    assert!(h.z < 0.0);
    // +Z only: nothing below the field.
    let z = field(FieldType::Force, |s| s.z_direction = ZDirection::Positive);
    assert_eq!(z.force(Vec3::new(0.0, 0.0, -1.0), Vec3::ZERO, 0.0), Vec3::ZERO);
    // Tube falloff: radial distance matters.
    let t = field(FieldType::Force, |s| {
        s.falloff_type = FalloffType::Tube;
        s.use_radial_max = true;
        s.radial_max = 1.0;
    });
    assert_eq!(t.force(Vec3::new(2.0, 0.0, 0.5), Vec3::ZERO, 0.0), Vec3::ZERO);
    assert!(t.force(Vec3::new(0.5, 0.0, 0.5), Vec3::ZERO, 0.0).length() > 0.0);
}

#[test]
fn field_objects_push_particles() {
    let (mut s, _) = emitter_scene(1);
    s.physics.gravity.use_gravity = false;
    let mut wind = Object::new("Wind", ObjectData::Empty);
    // Wind along +X: rotate the field's Z onto X.
    wind.transform.rotation = Quat::from_axis_angle(Vec3::Y, std::f32::consts::FRAC_PI_2);
    let mut f = FieldSettings::new(FieldType::Wind);
    f.strength = 5.0;
    wind.physics.field = Some(f);
    s.add_object(wind);
    play(&mut s, 20);
    let fr = particle_frame(&s, 20);
    let moved: Vec<f32> = fr.particles.iter().filter(|p| p.state == Alive::Alive && p.birth < 15.0).map(|p| p.vel.x).collect();
    assert!(!moved.is_empty() && moved.iter().all(|&x| x > 0.5), "{moved:?}");
    let eff = Effectors::collect(&s, None, &EffectorWeights::default());
    assert_eq!(eff.list.len(), 1);
}

fn smoke_scene(buoyancy: f32) -> (Scene, usize) {
    let mut s = Scene::new();
    let d = s.add_mesh_object("Domain", primitives::cube(2.0));
    let mut dom = FluidSettings::new(FluidType::Domain);
    dom.resolution_max = 16;
    dom.alpha = buoyancy;
    dom.beta = buoyancy;
    dom.vorticity = 0.0;
    s.objects[d].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(dom))));
    let fl = s.add_mesh_object("Flow", primitives::ico_sphere(1, 0.35));
    s.objects[fl].transform.location = Vec3::new(0.0, 0.0, -0.4);
    let mut f = FluidSettings::new(FluidType::Flow);
    f.flow_behavior = FlowBehavior::Inflow;
    s.objects[fl].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    (s, d)
}

fn fluid_frame(s: &Scene, f: i32) -> fluid::FluidFrame {
    match &*s.physics.caches.exact(&fluid::key("Domain", 0), f).expect("cached fluid") {
        SimFrame::Fluid(x) => x.clone(),
        _ => panic!(),
    }
}

/// Centre of mass height of the smoke.
fn smoke_height(f: &fluid::FluidFrame) -> f32 {
    let [nx, ny, nz] = f.res;
    let (mut m, mut mz) = (0.0f64, 0.0f64);
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let d = f.density[i + nx * (j + ny * k)] as f64;
                m += d;
                mz += d * k as f64;
            }
        }
    }
    (mz / m.max(1e-12)) as f32 * f.dx + f.min.z
}

#[test]
fn smoke_rises_with_buoyancy_and_conserves_density() {
    let (mut s, _) = smoke_scene(1.0);
    play(&mut s, 4);
    let early = fluid_frame(&s, 4);
    assert!(early.total_density() > 0.0, "the flow should emit smoke");
    let h0 = smoke_height(&early);
    play(&mut s, 24);
    let h1 = smoke_height(&fluid_frame(&s, 24));
    assert!(h1 > h0 + 0.1, "smoke should rise: {h0} -> {h1}");

    // No sources and no buoyancy: advection and projection keep the total.
    // Continue from the cached full state with the flow removed.
    let (mut q, d) = smoke_scene(0.0);
    play(&mut q, 3);
    let resume = q.physics.caches.resume(&fluid::key("Domain", 0)).expect("full state");
    let SimFrame::Fluid(full) = &*resume.1 else { panic!() };
    let start = full.total_density();
    let fl = q.objects.iter().position(|o| o.name == "Flow").unwrap();
    q.objects[fl].modifiers.clear();
    let Modifier::Physics(PhysicsModifier::Fluid(dom)) = q.objects[d].modifiers[0].clone() else { panic!() };
    let mut cur = full.clone();
    for _ in 0..20 {
        cur = fluid::step(&q, d, &dom, &cur).expect("step");
    }
    let end = cur.total_density();
    assert!(start > 0.0 && (end - start).abs() / start < 0.15, "density {start} -> {end}");
    assert!(fluid::test_divergence(&cur, &dom) < 1e-2, "projection left divergence");
}

#[test]
fn cache_invalidates_on_parameter_change_and_jump_back() {
    let mut s = rb_scene();
    ground(&mut s);
    let c = add_body(&mut s, "Cube", primitives::cube(1.0), Vec3::new(0.0, 0.0, 3.0), RigidBodyType::Active, CollisionShape::Box);
    play(&mut s, 20);
    let caches = s.physics.caches.clone();
    assert_eq!(caches.entry(rigid::CACHE_KEY).unwrap().frames.len(), 20);
    assert!(!caches.entry(rigid::CACHE_KEY).unwrap().outdated);
    // Going back reads the cache; nothing is stepped.
    s.frame_current = 10;
    assert_eq!(update(&s).stepped, 0);
    // Changing a setting frees the frames and marks the cache outdated.
    s.objects[c].physics.rigid_body.as_mut().unwrap().mass = 5.0;
    let r = update_limited(&s, 0);
    assert_eq!(r.invalidated, 1);
    let e = caches.entry(rigid::CACHE_KEY).unwrap();
    assert!(e.outdated, "{}", e.info());
    assert!(e.info().contains("outdated") && e.frames.len() <= 1);
    // Simulating up to the current frame again brings it up to date.
    update(&s);
    assert!(!caches.entry(rigid::CACHE_KEY).unwrap().outdated);
    s.objects[c].physics.rigid_body.as_mut().unwrap().mass = 4.0;
    update_limited(&s, 0);
    // Replaying from the start clears it.
    s.frame_current = 1;
    play(&mut s, 5);
    let e = caches.entry(rigid::CACHE_KEY).unwrap();
    assert!(!e.outdated && e.frames.len() == 5, "{}", e.info());
    // Moving the object at the start frame restarts the simulation there.
    s.objects[c].transform.location.z = 4.0;
    s.frame_current = 1;
    update(&s);
    assert_eq!(caches.entry(rigid::CACHE_KEY).unwrap().frames.len(), 1);
    assert!((z_of(&s, c) - 4.0).abs() < 1e-5);

    // Baking keeps frames through setting changes until freed.
    let n = bake(&mut s, &[], &mut |_, _| {}).unwrap();
    assert_eq!(n, 1);
    assert!(caches.is_baked(rigid::CACHE_KEY));
    let frames = caches.entry(rigid::CACHE_KEY).unwrap().frames.len();
    assert_eq!(frames as i32, 250);
    s.objects[c].physics.rigid_body.as_mut().unwrap().mass = 1.0;
    update(&s);
    assert_eq!(caches.entry(rigid::CACHE_KEY).unwrap().frames.len(), 250);
    free_bakes(&s, &[]);
    assert!(!caches.is_baked(rigid::CACHE_KEY));
    assert_eq!(caches.entry(rigid::CACHE_KEY).unwrap().frames.len(), 0);
}

#[test]
fn disk_cache_round_trip() {
    let mut s = rb_scene();
    ground(&mut s);
    add_body(&mut s, "Cube", primitives::cube(1.0), Vec3::new(0.0, 0.0, 3.0), RigidBodyType::Active, CollisionShape::Box);
    s.physics.rigidbody_world.as_mut().unwrap().point_cache.frame_end = 30;
    s.physics.rigidbody_world.as_mut().unwrap().point_cache.use_disk_cache = true;
    bake(&mut s, &[], &mut |_, _| {}).unwrap();
    let files = disk_files(&s);
    assert_eq!(files.len(), 1);
    s.frame_current = 30;
    let z = s.world_matrix(1).transform_point(Vec3::ZERO).z;
    let mut t = s.clone();
    t.physics.caches = Default::default();
    let n = load_disk_caches(&t, &|name| files.iter().find(|f| f.0 == name).map(|f| f.1.clone()));
    assert_eq!(n, 1);
    assert!(t.physics.caches.is_baked(rigid::CACHE_KEY));
    assert_eq!(t.world_matrix(1).transform_point(Vec3::ZERO).z, z);
    assert_eq!(free_bakes(&t, &[]), vec![files[0].0.clone()]);
}

#[test]
fn physics_round_trips_through_polyops() {
    let mut s = rb_scene();
    s.physics.gravity.gravity = [0.0, 0.0, -3.5];
    s.physics.rigidbody_world.as_mut().unwrap().substeps_per_frame = 20;
    ground(&mut s);
    let c = add_body(&mut s, "Cube", primitives::cube(1.0), Vec3::new(0.0, 0.0, 3.0), RigidBodyType::Active, CollisionShape::Sphere);
    {
        let rb = s.objects[c].physics.rigid_body.as_mut().unwrap();
        rb.mass = 2.5;
        rb.friction = 0.25;
        rb.use_deactivation = true;
    }
    let mut e = Object::new("Hinge", ObjectData::Empty);
    let mut con = RigidBodyConstraint { kind: ConstraintType::GenericSpring, object1: "Ground".into(), object2: "Cube".into(), use_breaking: true, breaking_threshold: 3.0, ..Default::default() };
    con.axes[3].use_limit = true;
    con.axes[3].upper = 0.5;
    con.axes[2].use_spring = true;
    e.physics.rigid_body_constraint = Some(con);
    let mut fs = FieldSettings::new(FieldType::Vortex);
    fs.strength = 4.0;
    fs.falloff_type = FalloffType::Cone;
    e.physics.field = Some(fs);
    s.add_object(e);
    let mut cl = ClothSettings::default();
    cl.quality = 7;
    cl.use_pressure = true;
    cl.effector_weights.gravity = 0.5;
    cl.point_cache.frame_end = 77;
    s.objects[c].modifiers.push(Modifier::Physics(PhysicsModifier::Cloth(Box::new(cl))));
    s.objects[c].modifiers.push(Modifier::Physics(PhysicsModifier::Collision(CollisionSettings { thickness_outer: 0.1, ..Default::default() })));
    let mut ps = ParticleSystem::default();
    ps.name = "Sparks".into();
    ps.seed = 11;
    ps.settings.count = 321;
    ps.settings.render_type = RenderType::Object;
    ps.settings.instance_object = "Ground".into();
    ps.settings.child_type = ChildType::Interpolated;
    s.objects[c].modifiers.push(Modifier::Physics(PhysicsModifier::ParticleSystem(Box::new(ps))));
    let mut fl = FluidSettings::new(FluidType::Flow);
    fl.flow_type = FlowType::Both;
    fl.smoke_color = [0.1, 0.2, 0.3];
    s.objects[0].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(fl))));
    s.objects[0].modifiers.push(Modifier::Physics(PhysicsModifier::SoftBody(Box::new(SoftBodySettings { goal_default: 0.3, ..Default::default() }))));

    let text = sex_3d::script::scene_to_polyops(&s);
    let mut t = Scene::new();
    t.objects.clear();
    sex_3d::script::run(&mut t, &text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    assert_eq!(t.physics.gravity, s.physics.gravity);
    assert_eq!(t.physics.rigidbody_world, s.physics.rigidbody_world);
    for o in &s.objects {
        let u = t.objects.iter().find(|x| x.name == o.name).unwrap_or_else(|| panic!("{} missing", o.name));
        assert_eq!(u.physics, o.physics, "{}", o.name);
        assert_eq!(u.modifiers, o.modifiers, "{}", o.name);
    }
}

#[test]
fn simulations_run_in_reasonable_time() {
    let t = Instant::now();
    let mut s = rb_scene();
    ground(&mut s);
    for k in 0..27 {
        let p = Vec3::new((k % 3) as f32 * 1.1 - 1.1, ((k / 3) % 3) as f32 * 1.1 - 1.1, 0.6 + (k / 9) as f32 * 1.1);
        add_body(&mut s, &format!("B{k}"), primitives::cube(1.0), p, RigidBodyType::Active, CollisionShape::Box);
    }
    play(&mut s, 100);
    let rigid_t = t.elapsed();
    let t = Instant::now();
    let mut c = Scene::new();
    let sph = c.add_mesh_object("Sphere", primitives::uv_sphere(32, 16, 1.0));
    c.objects[sph].modifiers.push(Modifier::Physics(PhysicsModifier::Collision(CollisionSettings::default())));
    let cl = cloth_grid(&mut c, 32, 3.0, 1.5);
    c.objects[cl].modifiers.push(Modifier::Physics(PhysicsModifier::Cloth(Box::default())));
    play(&mut c, 50);
    let cloth_t = t.elapsed();
    let t = Instant::now();
    let (mut sm, _) = smoke_scene(1.0);
    play(&mut sm, 30);
    let smoke_t = t.elapsed();
    eprintln!("27 boxes x100 frames: {rigid_t:?}, cloth 33x33 x50: {cloth_t:?}, smoke 16^3 x30: {smoke_t:?}");
    // Generous: debug-unoptimized CI would still pass the release budget.
    assert!(rigid_t.as_secs_f32() < 20.0);
    assert!(cloth_t.as_secs_f32() < 20.0);
    assert!(smoke_t.as_secs_f32() < 20.0);
}


// --- Rendering ---------------------------------------------------------------

use sex_3d::math::Mat4;
use sex_3d::render::{self, LightPaths, RenderSettings};
use sex_3d::scene::{Camera, Image, Transform};

fn look(s: &mut Scene, eye: Vec3, target: Vec3) {
    let mut o = Object::new("Camera", ObjectData::Camera(Camera { fov_y: 0.7, ..Camera::default() }));
    o.transform = Transform::from_matrix(&Mat4::look_at(eye, target, Vec3::Z).inverse());
    let c = s.add_object(o);
    s.active_camera = Some(c);
}

fn rs(w: u32, spp: u32) -> RenderSettings {
    RenderSettings { width: w, height: w, samples: spp, max_bounces: 2, clamp: 0.0, light_paths: LightPaths::unlimited(), ..RenderSettings::default() }
}

fn centre(img: &Image) -> [f32; 4] {
    let (w, h) = (img.width, img.height);
    let mut acc = [0.0f32; 4];
    let mut n = 0.0;
    for y in h / 2 - 2..h / 2 + 2 {
        for x in w / 2 - 2..w / 2 + 2 {
            let p = img.pixels[(y * w + x) as usize];
            for c in 0..4 {
                acc[c] += p[c];
            }
            n += 1.0;
        }
    }
    acc.map(|v| v / n)
}

#[test]
fn smoke_renders_as_a_heterogeneous_volume() {
    let (mut s, _) = smoke_scene(1.0);
    s.world.color = Vec3::splat(1.0);
    look(&mut s, Vec3::new(0.0, -7.0, 0.0), Vec3::ZERO);
    let fl = s.objects.iter().position(|o| o.name == "Flow").unwrap();
    s.objects[fl].visible = false;
    // Before any smoke the domain box is invisible.
    s.frame_current = 0;
    let empty = render::render(&s, &rs(24, 8)).color;
    assert!((centre(&empty)[0] - 1.0).abs() < 0.02, "empty domain: {:?}", centre(&empty));
    play(&mut s, 20);
    let smoky = render::render(&s, &rs(24, 32)).color;
    let c = centre(&smoky)[0];
    assert!(c < 0.97, "smoke should dim the background: {c}");
    // The top corner of the domain, where no smoke is, stays clear.
    let corner = smoky.pixels[(3 * 24 + 3) as usize][0];
    assert!(corner > c, "corner {corner} vs centre {c}");
}

#[test]
fn particles_render_as_object_instances_and_hair() {
    let mut s = Scene::new();
    s.world.color = Vec3::splat(1.0);
    let e = s.add_mesh_object("Emitter", primitives::plane(2.0));
    let ico = s.add_mesh_object("Ico", primitives::ico_sphere(1, 1.0));
    s.objects[ico].transform.location = Vec3::new(30.0, 0.0, 0.0);
    let mut sys = ParticleSystem::default();
    sys.settings.count = 30;
    sys.settings.frame_start = 1.0;
    sys.settings.frame_end = 2.0;
    sys.settings.lifetime = 100.0;
    sys.settings.physics_type = ParticlePhysics::None;
    sys.settings.render_type = RenderType::Object;
    sys.settings.instance_object = "Ico".into();
    sys.settings.particle_size = 0.2;
    sys.settings.use_render_emitter = false;
    s.objects[e].modifiers.push(Modifier::Physics(PhysicsModifier::ParticleSystem(Box::new(sys))));
    look(&mut s, Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO);
    let s_cam = s.active_camera.unwrap();
    s.objects[s_cam].transform = Transform::from_matrix(&Mat4::look_at(Vec3::new(0.0, 0.0, 8.0), Vec3::ZERO, Vec3::Y).inverse());
    play(&mut s, 3);
    let img = render::render(&s, &rs(32, 4)).color;
    let dark = img.pixels.iter().filter(|p| p[0] < 0.9).count();
    assert!(dark > 10, "instances should show: {dark} dark pixels");
    // Without instances and with the emitter hidden the view is clear.
    if let Modifier::Physics(PhysicsModifier::ParticleSystem(p)) = &mut s.objects[e].modifiers[0] {
        p.settings.render_type = RenderType::None;
    }
    let clear = render::render(&s, &rs(32, 4)).color;
    assert!(clear.pixels.iter().all(|p| p[0] > 0.95), "emitter should be hidden");
    // Hair renders as tubes.
    let mut h = ParticleSystem::default();
    h.settings = ParticleSettings::hair();
    h.settings.count = 400;
    h.settings.radius_scale = 0.05;
    s.objects[e].modifiers.push(Modifier::Physics(PhysicsModifier::ParticleSystem(Box::new(h))));
    let hair = render::render(&s, &rs(32, 4)).color;
    let dark = hair.pixels.iter().filter(|p| p[0] < 0.9).count();
    assert!(dark > 20, "hair should show: {dark}");
}
