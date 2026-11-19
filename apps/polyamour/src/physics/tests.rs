// SPDX-License-Identifier: GPL-3.0-only
use super::*;
use crate::editors::properties::{State, Tab};
use crate::editors::Editor;
use crate::input::{Button, Input, Mods};
use crate::App;
use sex_3d::physics::props::Value;

fn run(d: &mut Doc, id: &str, arg: &str) {
    d.run(&Op::with(id, arg));
    let q = core::mem::take(&mut d.queue);
    for op in q {
        d.run(&op);
    }
}

fn cube(d: &Doc) -> usize {
    d.scene.find_object("Cube").expect("default cube")
}

/// Tick until the simulations reach the current frame.
fn settle(d: &mut Doc) {
    for _ in 0..200 {
        crate::anim::tick(d);
        if !pending(d) {
            break;
        }
    }
    assert!(!pending(d));
}

fn height(d: &Doc, i: usize) -> f32 {
    d.scene.world_matrix(i).transform_point(Vec3::ZERO).z
}

#[test]
fn toggles_add_and_remove() {
    let mut d = Doc::default();
    let c = cube(&d);
    run(&mut d, "physics.toggle", "CLOTH");
    assert!(matches!(d.scene.objects[c].modifiers.last(), Some(Modifier::Physics(PhysicsModifier::Cloth(_)))));
    assert!(has_kind(&d.scene.objects[c], "CLOTH"));
    run(&mut d, "physics.toggle", "CLOTH");
    assert!(!has_kind(&d.scene.objects[c], "CLOTH"));
    for k in ["FORCE_FIELD", "COLLISION", "SOFT_BODY", "FLUID", "DYNAMIC_PAINT", "RIGID_BODY", "RIGID_BODY_CONSTRAINT"] {
        run(&mut d, "physics.toggle", k);
        assert!(has_kind(&d.scene.objects[c], k), "{k}");
    }
    assert!(d.scene.physics.rigidbody_world.is_some(), "a rigid body brings the world");
    run(&mut d, "ed.undo", "");
    run(&mut d, "ed.undo", "");
    assert!(d.scene.objects[c].physics.rigid_body.is_none() && d.scene.objects[c].physics.rigid_body_constraint.is_none());
}

#[test]
fn set_properties_by_text() {
    let mut d = Doc::default();
    let c = cube(&d);
    run(&mut d, "physics.toggle", "RIGID_BODY");
    let t = Target::RigidBody(c).encode();
    run(&mut d, "physics.set", &format!("{t}\tfriction\t0.25"));
    run(&mut d, "physics.set", &format!("{t}\tcollision_shape\tSPHERE"));
    let rb = d.scene.objects[c].physics.rigid_body.clone().unwrap();
    assert_eq!((rb.friction, rb.collision_shape), (0.25, CollisionShape::Sphere));
    run(&mut d, "physics.set", "gravity\tgravity\t0,0,-2");
    assert_eq!(d.scene.physics.gravity.gravity, [0.0, 0.0, -2.0]);
    run(&mut d, "ed.undo", "");
    assert_eq!(d.scene.physics.gravity.gravity, [0.0, 0.0, -9.81]);
    assert_eq!(Target::parse("mod:3:1"), Some(Target::Modifier(3, 1)));
    assert_eq!(with_props(&mut d.scene, Target::RigidBody(c), |p| props::get(p, "friction")), Some(Some(Value::F(0.25))));
}

#[test]
fn frames_step_the_simulation() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.scene.objects[c].transform.location = Vec3::new(0.0, 0.0, 10.0);
    run(&mut d, "rigidbody.objects_add", "ACTIVE");
    settle(&mut d);
    let z0 = height(&d, c);
    d.frame = 30;
    crate::anim::tick(&mut d);
    assert!(pending(&d), "a far jump steps a few frames per tick");
    settle(&mut d);
    let z1 = height(&d, c);
    assert!(z1 < z0 - 2.0, "the cube falls: {z0} -> {z1}");
    // Back before the start: the unsimulated scene.
    d.frame = 1;
    settle(&mut d);
    assert!((height(&d, c) - 10.0).abs() < 1e-3);
}

#[test]
fn bake_free_and_disk_cache() {
    let dir = std::env::temp_dir().join(format!("polyamour-physics-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut d = Doc::default();
    let c = cube(&d);
    d.scene.objects[c].transform.location = Vec3::new(0.0, 0.0, 5.0);
    run(&mut d, "rigidbody.objects_add", "ACTIVE");
    {
        let w = d.scene.physics.rigidbody_world.as_mut().unwrap();
        w.point_cache.frame_end = 20;
        w.point_cache.use_disk_cache = true;
    }
    d.path = Some(dir.join("drop.poly").to_string_lossy().into_owned());
    run(&mut d, "ptcache.bake", ph::rigid::CACHE_KEY);
    assert!(d.report.starts_with("Baked 1 simulation"), "{}", d.report);
    let (_, baked, _, frames) = d.scene.physics.caches.status(ph::rigid::CACHE_KEY);
    assert!(baked && frames >= 20);
    let files: Vec<_> = std::fs::read_dir(dir.join("cache")).unwrap().collect();
    assert_eq!(files.len(), 1);
    // Saved and opened again: the bake comes back from the cache folder.
    let text = crate::persist::save(&d);
    let mut e = crate::persist::load(&text).unwrap();
    e.path = d.path.clone();
    assert_eq!(load_disk_caches(&e), 1);
    assert!(e.scene.physics.caches.is_baked(ph::rigid::CACHE_KEY));
    e.scene.frame_current = 20;
    d.scene.frame_current = 20;
    assert!((e.scene.world_matrix(c).transform_point(Vec3::ZERO).z - height(&d, c)).abs() < 1e-5);
    run(&mut d, "ptcache.free_bake_all", "");
    assert!(!d.scene.physics.caches.is_baked(ph::rigid::CACHE_KEY));
    assert_eq!(std::fs::read_dir(dir.join("cache")).unwrap().count(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_round_trip_and_gravity_migration() {
    let mut d = Doc::default();
    let c = cube(&d);
    run(&mut d, "physics.toggle", "CLOTH");
    run(&mut d, "physics.toggle", "FORCE_FIELD");
    run(&mut d, "rigidbody.objects_add", "PASSIVE");
    run(&mut d, "object.particle_system_add", "");
    run(&mut d, "physics.set", &format!("{}\tquality\t9", Target::Modifier(c, 0).encode()));
    run(&mut d, "physics.set", &format!("{}\ttype\tWIND", Target::Field(c).encode()));
    run(&mut d, "physics.set", "gravity\tgravity\t0,0,-3");
    let text = crate::persist::save(&d);
    let e = crate::persist::load(&text).unwrap();
    assert_eq!(e.scene.physics.gravity, d.scene.physics.gravity);
    assert_eq!(e.scene.physics.rigidbody_world, d.scene.physics.rigidbody_world);
    assert_eq!(e.scene.objects[c].physics, d.scene.objects[c].physics);
    assert_eq!(e.scene.objects[c].modifiers, d.scene.objects[c].modifiers);
    // An older file kept gravity with the editor settings.
    let old = crate::persist::save(&Doc::default()) + "#@set gravity_on\t0\n#@set gravity\t0,0,-1.5\n";
    let e = crate::persist::load(&old).unwrap();
    assert!(!e.scene.physics.gravity.use_gravity);
    assert_eq!(e.scene.physics.gravity.gravity, [0.0, 0.0, -1.5]);
}

#[test]
fn rigid_body_menu_operators() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.scene.objects[c].transform.location = Vec3::new(0.0, 0.0, 4.0);
    run(&mut d, "rigidbody.objects_add", "ACTIVE");
    run(&mut d, "rigidbody.shape_change", "SPHERE");
    run(&mut d, "rigidbody.mass_calculate", "1000");
    let rb = d.scene.objects[c].physics.rigid_body.clone().unwrap();
    assert_eq!(rb.collision_shape, CollisionShape::Sphere);
    // Blender measures the collision shape: a unit sphere of 1000 kg/m³.
    assert!((rb.mass - 4000.0 / 3.0 * std::f32::consts::PI).abs() < 1.0, "{}", rb.mass);
    // Connect the cube to a second one.
    run(&mut d, "mesh.primitive_add", "cube");
    let b = d.active().unwrap();
    d.scene.selected = vec![b, c];
    d.scene.active_object = Some(c);
    run(&mut d, "rigidbody.connect", "HINGE");
    let e = d.scene.objects.len() - 1;
    let con = d.scene.objects[e].physics.rigid_body_constraint.clone().unwrap();
    assert_eq!((con.kind, con.object1.as_str()), (ConstraintType::Hinge, "Cube"));
    assert!(matches!(d.scene.objects[e].data, ObjectData::Empty));
    // Bake to keyframes: the fall becomes animation, the rigid body goes
    // (without the hinge, which holds the cube up).
    d.scene.objects[e].physics.rigid_body_constraint = None;
    d.scene.physics.rigidbody_world.as_mut().unwrap().point_cache.frame_end = 15;
    d.scene.selected = vec![c];
    run(&mut d, "rigidbody.bake_to_keyframes", "");
    assert!(d.scene.objects[c].physics.rigid_body.is_none());
    d.frame = 15;
    crate::anim::tick(&mut d);
    assert!(d.scene.objects[c].transform.location.z < 3.9, "keyed fall: {}", d.scene.objects[c].transform.location.z);
    run(&mut d, "rigidbody.objects_remove", "");
}

#[test]
fn particles_and_quick_effects() {
    let mut d = Doc::default();
    let c = cube(&d);
    run(&mut d, "object.particle_system_add", "");
    run(&mut d, "object.particle_system_add", "");
    assert_eq!(particle_slots(&d.scene.objects[c]).len(), 2);
    assert_eq!(active_particle(&d, c), 1);
    run(&mut d, "object.particle_system_remove", "");
    assert_eq!(particle_slots(&d.scene.objects[c]).len(), 1);
    run(&mut d, "object.quick_fur", "");
    let slots = particle_slots(&d.scene.objects[c]);
    let Modifier::Physics(PhysicsModifier::ParticleSystem(p)) = &d.scene.objects[c].modifiers[slots[1]] else { panic!() };
    assert_eq!(p.settings.kind, ph::ParticleType::Hair);
    d.select_only(Some(c));
    run(&mut d, "object.quick_smoke", "");
    let dom = d.active().unwrap();
    assert_eq!(d.scene.objects[dom].name, "Smoke Domain");
    assert_eq!(ph::fluid::domains(&d.scene).len(), 1);
    assert!(d.scene.objects[c].modifiers.iter().any(|m| matches!(m, Modifier::Physics(PhysicsModifier::Fluid(f)) if f.fluid_type == FluidType::Flow)));
    // The domain holds the flow object.
    let m = d.scene.world_matrix(dom);
    assert!(m.transform_point(Vec3::new(-1.0, -1.0, -1.0)).z < -1.0 && m.transform_point(Vec3::ONE).z > 1.0);
}

#[test]
fn demo_has_every_simulation() {
    let mut d = Doc::default();
    demo(&mut d);
    let s = &d.scene;
    let keys: Vec<String> = ph::sims(s).into_iter().map(|x| x.key).collect();
    for k in ["rigidbody_world", "Cloth/cloth0", "Emitter/particles0", "Smoke Domain/fluid0"] {
        assert!(keys.iter().any(|x| x == k), "{k} in {keys:?}");
    }
    let em = s.find_object("Emitter").unwrap();
    let (slot, _, sys) = ph::particles::systems(s, em)[0];
    assert!(ph::particles::view(s, em, slot, sys, false).points.len() > 100);
    let b = s.find_object("Box.004").unwrap();
    assert!(height(&d, b) < 6.0, "the top box fell");
}

fn props_mut(app: &mut App) -> &mut State {
    for a in app.workspaces[app.active_ws].areas_mut() {
        if let Editor::Properties(s) = &mut a.editor {
            return s;
        }
    }
    panic!("no properties editor")
}

fn click(app: &mut App, r: crate::draw::Rect) {
    let (x, y) = r.center();
    app.frame(Input::Move { x, y });
    app.frame(Input::Down { x, y, button: Button::Left, mods: Mods::NONE });
    app.frame(Input::Up { x, y, button: Button::Left });
}

#[test]
fn physics_tab_buttons_and_panels() {
    let mut app = App::new(1600, 900);
    props_mut(&mut app).tab = Tab::Physics;
    app.frame(Input::Tick);
    let c = cube(&app.doc);
    let r = props_mut(&mut app).field("Cloth").expect("Cloth button");
    click(&mut app, r);
    assert!(has_kind(&app.doc.scene.objects[c], "CLOTH"));
    app.frame(Input::Tick);
    assert!(props_mut(&mut app).field("Vertex Mass").is_some(), "cloth panels drawn");
    let r = props_mut(&mut app).field("Rigid Body").unwrap();
    click(&mut app, r);
    app.frame(Input::Tick);
    assert!(props_mut(&mut app).field("Friction").is_some());
    // Dynamic Paint: the modifier and its Canvas toggle.
    let r = props_mut(&mut app).field("Dynamic Paint").unwrap();
    click(&mut app, r);
    assert!(has_kind(&app.doc.scene.objects[c], "DYNAMIC_PAINT"));
    app.frame(Input::Tick);
    // The Canvas checkbox sits below the fold; toggle it as the button does.
    assert!(props_mut(&mut app).field("Canvas").is_some(), "Canvas toggle drawn");
    run(&mut app.doc, "dpaint.type_toggle", "CANVAS");
    app.frame(Input::Tick);
    let on = app.doc.scene.objects[c].modifiers.iter().any(|m| matches!(m, Modifier::Physics(PhysicsModifier::DynamicPaint(d)) if d.use_canvas));
    assert!(on, "the Canvas toggle sets use_canvas");
    assert!(props_mut(&mut app).field("Surface Type").is_some(), "canvas panels drawn");
    // Scene tab: gravity and the world.
    props_mut(&mut app).tab = Tab::Scene;
    app.frame(Input::Tick);
    assert!(props_mut(&mut app).field("Gravity X").is_some());
    props_mut(&mut app).tab = Tab::Particles;
    app.frame(Input::Tick);
    run(&mut app.doc, "object.particle_system_add", "");
    app.frame(Input::Tick);
    assert!(props_mut(&mut app).field("Number").is_some());
}

#[test]
fn quick_liquid_bake_and_free() {
    let mut d = Doc::default();
    let c = cube(&d);
    d.scene.objects[c].transform.scale = Vec3::splat(0.5);
    d.select_only(Some(c));
    run(&mut d, "object.quick_liquid", "");
    let dom = d.active().unwrap();
    assert_eq!(d.scene.objects[dom].name, "Liquid Domain");
    let Some(Modifier::Physics(PhysicsModifier::Fluid(f))) = d.scene.objects[dom].modifiers.last_mut() else { panic!() };
    assert_eq!(f.domain_type, DomainType::Liquid);
    assert!(f.closed_borders().iter().all(|&b| b), "Quick Liquid closes the borders");
    f.resolution_max = 12;
    f.cache_frame_end = 6;
    assert!(!d.scene.objects[c].visible, "the flow hides");
    let key = ph::fluid::key("Liquid Domain", d.scene.objects[dom].modifiers.len() - 1);
    run(&mut d, "fluid.bake_all", "");
    assert!(d.report.starts_with("Baked 1"), "{}", d.report);
    let (_, baked, _, frames) = d.scene.physics.caches.status(&key);
    assert!(baked && frames == 6);
    d.scene.frame_current = 4;
    let m = d.scene.evaluated_mesh(dom).unwrap();
    assert!(m.faces.len() > 20, "the domain shows the liquid");
    run(&mut d, "fluid.free_all", "");
    assert_eq!(d.scene.physics.caches.status(&key).3, 0);
    // The Physics tab draws the liquid panels.
    let mut app = App::new(1200, 900);
    app.doc = d;
    props_mut(&mut app).tab = Tab::Physics;
    app.frame(Input::Tick);
    assert!(props_mut(&mut app).field("Domain Type").is_some());
}

#[test]
fn liquid_demo_runs() {
    let mut d = Doc::default();
    liquid_demo(&mut d);
    let dom = d.scene.find_object("Liquid Domain").unwrap();
    let m = d.scene.evaluated_mesh(dom).unwrap();
    assert!(m.faces.len() > 500, "liquid surface at frame 24");
    let pool = d.scene.find_object("Pool").unwrap();
    let pm = d.scene.evaluated_mesh(pool).unwrap();
    assert!(pm.positions.iter().any(|p| p.z.abs() > 1e-4), "the ball ripples the pool");
    assert!(pm.positions.iter().all(|p| p.z.abs() < 0.3), "the waves stay stable");
}
