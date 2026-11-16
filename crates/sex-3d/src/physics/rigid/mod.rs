// SPDX-License-Identifier: GPL-3.0-only
//! The Rigid Body World: every object with a rigid body, simulated frame by
//! frame into the point cache, and read back as the objects' world
//! transforms (Blender's rigid body sync, after constraints).
//!
//! Deviations from Blender's Bullet: collision shapes sit at the object
//! origin like Blender's (bounds-sized boxes, spheres, capsules, cylinders
//! and cones; cylinders and cones are 16-sided polytopes); an *active* Mesh
//! shape collides as its convex hull (passive meshes use their triangles);
//! the margin only widens contact detection; every object with a rigid body
//! belongs to the world (objects here live in one collection, so Blender's
//! world collection is not used to filter).
pub mod collide;
pub mod shape;
pub mod solver;

use crate::math::{Mat4, Quat, Vec3};
use crate::physics::cache::SimFrame;
use crate::physics::settings::{CollisionShape, MeshSource, RigidBody, RigidBodyType};
use crate::scene::{ObjectData, Scene};
use collide::Pose;
use shape::{Polytope, Shape, TriMesh};
use solver::{Body, Joint, WarmCache, World};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

pub const CACHE_KEY: &str = "rigidbody_world";

/// One body's state in a cached frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    pub ang: Vec3,
    pub sleeping: bool,
    pub sleep_time: f32,
}

/// A cached frame of the world.
#[derive(Clone, Debug, Default)]
pub struct RigidFrame {
    pub bodies: HashMap<String, BodyState>,
    pub broken: Vec<String>,
    pub warm: Arc<WarmCache>,
}

impl RigidFrame {
    pub fn bytes(&self) -> usize {
        self.bodies.len() * 80 + 64
    }
}

/// Is object `i` moved by the simulation (active, dynamic, not animated)?
pub fn is_simulated(rb: &RigidBody) -> bool {
    rb.kind == RigidBodyType::Active && rb.enabled && !rb.kinematic
}

/// The simulated world matrix of object `idx` at the scene's frame, if the
/// rigid body world moves it.
pub fn override_matrix(scene: &Scene, idx: usize, base: &Mat4) -> Option<Mat4> {
    let w = scene.physics.rigidbody_world.as_ref()?;
    if !w.enabled || scene.frame_current < w.point_cache.frame_start {
        return None;
    }
    let o = &scene.objects[idx];
    if !is_simulated(o.physics.rigid_body.as_ref()?) {
        return None;
    }
    let f = scene.physics.caches.frame_at(CACHE_KEY, scene.frame_current.min(w.point_cache.frame_end))?;
    let SimFrame::Rigid(rf) = &*f else { return None };
    let st = rf.bodies.get(&o.name)?;
    let scale = base.mat3().scale();
    Some(Mat4::from_trs(st.pos, st.rot, scale))
}

fn shape_cache() -> &'static Mutex<HashMap<u64, Arc<Shape>>> {
    static S: OnceLock<Mutex<HashMap<u64, Arc<Shape>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The mesh a rigid body collides with, in object space.
fn body_mesh(scene: &Scene, i: usize, src: MeshSource) -> Option<crate::mesh::Mesh> {
    let ObjectData::Mesh(mi) = scene.objects[i].data else { return None };
    match src {
        MeshSource::Base => Some(scene.meshes[mi].clone()),
        _ => scene.evaluated_mesh(i),
    }
}

/// Build (or fetch) the collision shape of object `i` scaled by `scale`.
pub fn build_shape(scene: &Scene, i: usize, rb: &RigidBody, scale: Vec3) -> Arc<Shape> {
    let mesh = body_mesh(scene, i, rb.mesh_source);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (rb.collision_shape as u8, rb.kind == RigidBodyType::Passive).hash(&mut h);
    scale.to_array().map(f32::to_bits).hash(&mut h);
    if let Some(m) = &mesh {
        m.positions.len().hash(&mut h);
        for p in &m.positions {
            p.to_array().map(f32::to_bits).hash(&mut h);
        }
        m.faces.len().hash(&mut h);
        for f in &m.faces {
            f.verts.hash(&mut h);
        }
    }
    let key = h.finish();
    if let Some(s) = shape_cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return s;
    }
    let pts: Vec<Vec3> = mesh.as_ref().map(|m| m.positions.iter().map(|p| p.mul_elem(scale)).collect()).unwrap_or_default();
    let (size, _center) = if pts.is_empty() {
        (scale * 2.0, Vec3::ZERO)
    } else {
        let mut b = crate::math::Aabb::EMPTY;
        for &p in &pts {
            b.grow(p);
        }
        (b.size(), b.center())
    };
    let half = size * 0.5;
    let rxy = half.x.max(half.y).max(1e-3);
    let shape = match rb.collision_shape {
        CollisionShape::Box => Shape::Poly(Arc::new(Polytope::cuboid(half))),
        CollisionShape::Sphere => Shape::Sphere(half.max_elem().max(1e-3)),
        CollisionShape::Capsule => Shape::Capsule(rxy, (half.z - rxy).max(0.0)),
        CollisionShape::Cylinder => Shape::Poly(Arc::new(Polytope::round(rxy, rxy, half.z.max(1e-3), 16))),
        CollisionShape::Cone => Shape::Poly(Arc::new(Polytope::round(rxy, 0.0, half.z.max(1e-3), 16))),
        CollisionShape::ConvexHull | CollisionShape::Mesh => {
            let passive_mesh = rb.collision_shape == CollisionShape::Mesh && !is_simulated(rb);
            match &mesh {
                Some(m) if passive_mesh && !m.faces.is_empty() => {
                    let tris = m.triangulate().iter().map(|t| t.verts).collect();
                    Shape::Mesh(Arc::new(TriMesh::new(pts.clone(), tris)))
                }
                _ => {
                    let mut use_pts = pts.clone();
                    if use_pts.len() > 4000 {
                        let step = use_pts.len() / 4000 + 1;
                        use_pts = use_pts.into_iter().step_by(step).collect();
                    }
                    match Polytope::hull(&use_pts) {
                        Some(p) => Shape::Poly(Arc::new(p)),
                        None => Shape::Poly(Arc::new(Polytope::cuboid(half.max(Vec3::splat(rb.collision_margin.max(0.01)))))),
                    }
                }
            }
        }
    };
    let s = Arc::new(shape);
    if let Ok(mut c) = shape_cache().lock() {
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, s.clone());
    }
    s
}

fn hash_f(h: &mut impl Hasher, v: &[f32]) {
    for x in v {
        x.to_bits().hash(h);
    }
}

/// The settings signature (no transforms) and the start-state signature
/// (transforms of everything taking part).
pub fn signatures(scene: &Scene) -> (u64, u64) {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut hi = std::collections::hash_map::DefaultHasher::new();
    let Some(w) = &scene.physics.rigidbody_world else { return (0, 0) };
    let mut w = w.clone();
    crate::physics::props::hash_into(&mut w, &mut h);
    hash_f(&mut h, &scene.physics.gravity.gravity);
    scene.physics.gravity.use_gravity.hash(&mut h);
    scene.fps.to_bits().hash(&mut h);
    for (i, o) in scene.objects.iter().enumerate() {
        if let Some(rb) = &o.physics.rigid_body {
            o.name.hash(&mut h);
            let mut rb = rb.clone();
            crate::physics::props::hash_into(&mut rb, &mut h);
            if let ObjectData::Mesh(mi) = o.data {
                let m = &scene.meshes[mi];
                m.positions.len().hash(&mut h);
                for p in &m.positions {
                    hash_f(&mut h, &p.to_array());
                }
            }
            hash_f(&mut hi, &scene.base_world_matrix(i).to_cols_array());
        }
        if let Some(c) = &o.physics.rigid_body_constraint {
            o.name.hash(&mut h);
            let mut c = c.clone();
            crate::physics::props::hash_into(&mut c, &mut h);
            hash_f(&mut hi, &scene.world_matrix(i).to_cols_array());
        }
        if let Some(f) = &o.physics.field {
            let mut f = f.clone();
            crate::physics::props::hash_into(&mut f, &mut h);
        }
    }
    (h.finish(), hi.finish())
}

/// Build the solver world for a step, from the scene and the previous frame.
fn build_world(scene: &Scene, prev: Option<&RigidFrame>) -> (World, Vec<usize>) {
    let w = scene.physics.rigidbody_world.as_ref().expect("rigid body world");
    let g = &scene.physics.gravity;
    let gravity = if g.use_gravity { Vec3::from_array(g.gravity) * w.effector_weights.gravity } else { Vec3::ZERO };
    let mut bodies = Vec::new();
    let mut objs = Vec::new();
    for (i, o) in scene.objects.iter().enumerate() {
        let Some(rb) = &o.physics.rigid_body else { continue };
        let base = scene.base_world_matrix(i);
        let (pos, rot, scale) = base.to_trs();
        let rot = rot.normalize();
        let shape = build_shape(scene, i, rb, scale.abs());
        let dynamic = is_simulated(rb);
        let inertia = shape.inertia(rb.mass.max(1e-3));
        let inv = |x: f32| if x > 1e-12 { 1.0 / x } else { 0.0 };
        let st = prev.and_then(|p| p.bodies.get(&o.name)).copied();
        let (pose, vel, ang, sleeping, sleep_time) = match (dynamic, st) {
            (true, Some(s)) => (Pose::new(s.pos, s.rot), s.vel, s.ang, s.sleeping, s.sleep_time),
            (true, None) => (Pose::new(pos, rot), Vec3::ZERO, Vec3::ZERO, rb.use_deactivation && rb.use_start_deactivated, 0.0),
            (false, Some(s)) => (Pose::new(s.pos, s.rot), Vec3::ZERO, Vec3::ZERO, false, 0.0),
            (false, None) => (Pose::new(pos, rot), Vec3::ZERO, Vec3::ZERO, false, 0.0),
        };
        bodies.push(Body {
            name: o.name.clone(),
            shape,
            inv_mass: if dynamic { inv(rb.mass) } else { 0.0 },
            inv_inertia: if dynamic { Vec3::new(inv(inertia.x), inv(inertia.y), inv(inertia.z)) } else { Vec3::ZERO },
            pose,
            vel,
            ang,
            dynamic,
            target: if dynamic { None } else { Some((pos, rot)) },
            friction: rb.friction,
            restitution: rb.restitution,
            lin_damp: rb.linear_damping,
            ang_damp: rb.angular_damping,
            margin: if rb.use_margin { rb.collision_margin } else { 0.0 },
            deactivation: rb.use_deactivation.then_some((rb.deactivate_linear_velocity, rb.deactivate_angular_velocity)),
            sleeping,
            sleep_time,
            force: Vec3::ZERO,
        });
        objs.push(i);
    }
    let by_name: HashMap<&str, usize> = bodies.iter().enumerate().map(|(k, b)| (b.name.as_str(), k)).collect();
    let broken: HashSet<&str> = prev.map(|p| p.broken.iter().map(|s| s.as_str()).collect()).unwrap_or_default();
    let mut joints = Vec::new();
    let mut no_collide = HashSet::new();
    for (i, o) in scene.objects.iter().enumerate() {
        let Some(c) = &o.physics.rigid_body_constraint else { continue };
        let a = by_name.get(c.object1.as_str()).copied();
        let b = by_name.get(c.object2.as_str()).copied();
        if a.is_none() && b.is_none() {
            continue;
        }
        let m = scene.world_matrix(i);
        let (pivot, frame, _) = m.to_trs();
        let frame = frame.normalize();
        // Anchors from the start state: the constraint keeps the bodies where
        // they began relative to each other.
        let local = |k: Option<usize>| -> (Vec3, Quat) {
            match k {
                Some(k) => {
                    let ob = objs[k];
                    let (bp, bq, _) = scene.base_world_matrix(ob).to_trs();
                    let bq = bq.normalize();
                    let pose = Pose::new(bp, bq);
                    (pose.inv_point(pivot), (bq.conjugate() * frame).normalize())
                }
                None => (pivot, frame),
            }
        };
        let (anchor_a, frame_a) = local(a);
        let (anchor_b, frame_b) = local(b);
        if c.disable_collisions {
            if let (Some(x), Some(y)) = (a, b) {
                no_collide.insert((x.min(y), x.max(y)));
            }
        }
        joints.push(Joint { name: o.name.clone(), a, b, settings: c.clone(), anchor_a, anchor_b, frame_a, frame_b, broken: broken.contains(o.name.as_str()), impulses: Vec::new() });
    }
    let iterations = w.solver_iterations.max(1) as u32;
    let warm = prev.map(|p| (*p.warm).clone()).unwrap_or_default();
    (World { bodies, joints, gravity, iterations, split_impulse: w.use_split_impulse, no_collide, warm }, objs)
}

/// The frame at the start of the simulation: every body where the scene
/// puts it.
pub fn initial_frame(scene: &Scene) -> RigidFrame {
    let (world, _) = build_world(scene, None);
    let mut bodies = HashMap::new();
    for b in &world.bodies {
        bodies.insert(b.name.clone(), BodyState { pos: b.pose.pos, rot: b.pose.rot, vel: Vec3::ZERO, ang: Vec3::ZERO, sleeping: b.sleeping, sleep_time: 0.0 });
    }
    RigidFrame { bodies, broken: Vec::new(), warm: Arc::new(WarmCache::default()) }
}

/// Advance one frame from `prev`, with kinematic bodies moving to where the
/// scene puts them now.
pub fn step_frame(scene: &Scene, prev: &RigidFrame) -> RigidFrame {
    let w = scene.physics.rigidbody_world.as_ref().expect("rigid body world");
    let (mut world, objs) = build_world(scene, Some(prev));
    let substeps = w.substeps_per_frame.max(1) as usize;
    let dt_frame = w.time_scale.max(0.0) / scene.fps.max(1e-3);
    let dt = dt_frame / substeps as f32;
    let start: Vec<(Vec3, Quat)> = world.bodies.iter().map(|b| (b.pose.pos, b.pose.rot)).collect();
    // Field forces, sampled at the start of the frame.
    let effectors = crate::physics::field::Effectors::collect(scene, None, &w.effector_weights);
    if dt > 0.0 {
        for s in 0..substeps {
            if !effectors.is_empty() {
                for (k, b) in world.bodies.iter_mut().enumerate() {
                    if b.dynamic {
                        let mut f = Vec3::ZERO;
                        for e in &effectors.list {
                            if e.object == objs[k] {
                                continue;
                            }
                            f += e.force(b.pose.pos, b.vel, scene.frame_current as f32) * effectors.weights.of(e.settings.kind);
                        }
                        b.force = f;
                    }
                }
            }
            world.step(dt, (s + 1) as f32 / substeps as f32, &start);
        }
    }
    let mut bodies = HashMap::new();
    for b in &world.bodies {
        bodies.insert(b.name.clone(), BodyState { pos: b.pose.pos, rot: b.pose.rot, vel: b.vel, ang: b.ang, sleeping: b.sleeping, sleep_time: b.sleep_time });
    }
    let broken = world.joints.iter().filter(|j| j.broken).map(|j| j.name.clone()).collect();
    RigidFrame { bodies, broken, warm: Arc::new(world.warm) }
}

/// Blender's Calculate Mass: volume (of the collision shape's mesh) times
/// `density` (kg/m³).
pub fn calculate_mass(scene: &Scene, i: usize, density: f32) -> Option<f32> {
    let mut m = scene.evaluated_mesh(i)?;
    let (_, _, s) = scene.world_matrix(i).to_trs();
    for p in &mut m.positions {
        *p = p.mul_elem(s);
    }
    let rb = scene.objects[i].physics.rigid_body.as_ref()?;
    let size = m.bounds().size();
    let half = size * 0.5;
    let rxy = half.x.max(half.y);
    let pi = std::f32::consts::PI;
    let v = match rb.collision_shape {
        CollisionShape::Box => size.x * size.y * size.z,
        CollisionShape::Sphere => 4.0 / 3.0 * pi * half.max_elem().powi(3),
        CollisionShape::Capsule => pi * rxy * rxy * ((half.z - rxy).max(0.0) * 2.0 + 4.0 / 3.0 * rxy),
        CollisionShape::Cylinder => pi * rxy * rxy * size.z,
        CollisionShape::Cone => pi * rxy * rxy * size.z / 3.0,
        _ => m.volume().abs(),
    };
    Some((v * density).max(0.001))
}
