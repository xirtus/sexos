// SPDX-License-Identifier: GPL-3.0-only
//! Physics data of `.blend` files: the rigid body world, rigid bodies and
//! constraints, force fields and collision settings (`PartDeflect`), and
//! the Cloth, Soft Body, Collision, Particle System and Fluid modifiers.
//! Caches are not read (bakes are redone or read from our own disk cache).
//!
//! DNA members and values used (from `DNA_rigidbody_types.h`,
//! `DNA_object_force_types.h`, `DNA_cloth_types.h`,
//! `DNA_particle_types.h`, `DNA_fluid_types.h`, `DNA_pointcache_types.h`):
//! - `Scene.rigidbody_world` → `RigidBodyWorld`: `flag` (1 muted,
//!   4 split impulse), `substeps_per_frame`, `num_solver_iterations`,
//!   `time_scale`, `effector_weights`, `group`, `constraints`, and the
//!   `PointCache` (`pointcache`, or `shared->pointcache` since 2.8):
//!   `startframe`, `endframe`, `flag` (64 disk cache).
//! - `Object.rigidbody_object` → `RigidBodyOb`: `type` (0 active,
//!   1 passive), `shape` (0 box, 1 sphere, 2 capsule, 3 cylinder, 4 cone,
//!   5 convex hull, 6 mesh), `mesh_source` (0 deform, 1 final, 2 base),
//!   `flag` (1 kinematic, 8 deactivation, 16 start deactivated,
//!   32 disabled, 64 margin), `mass`, `friction`, `restitution`, `margin`,
//!   `lin_damping`, `ang_damping`, `lin_sleep_thresh`, `ang_sleep_thresh`.
//! - `Object.rigidbody_constraint` → `RigidBodyCon`: `ob1`, `ob2`, `type`
//!   (0 point, 1 fixed, 2 hinge, 3 slider, 4 piston, 5 generic, 6 generic
//!   spring, 7 motor), `flag` (1 enabled, 4 disable collisions, 8 breaking,
//!   16 override iterations, 32.. limit lin x/y/z, 256.. limit ang x/y/z,
//!   2048.. spring x/y/z, 16384.. spring ang x/y/z, 1<<17 motor lin,
//!   1<<18 motor ang), `breaking_threshold`, `num_solver_iterations`,
//!   `spring_type`, `limit_{lin,ang}_{x,y,z}_{lower,upper}`,
//!   `spring_{stiffness,damping}_{x,y,z,ang_x,ang_y,ang_z}`,
//!   `motor_{lin,ang}_{target_velocity,max_impulse}`.
//! - `Object.pd` → `PartDeflect`: `forcefield` (1 force, 2 vortex,
//!   3 magnetic, 4 wind, 7 harmonic, 8 charge, 11 turbulence, 12 drag),
//!   `shape` (0 point, 1 plane), `falloff` (0 sphere, 1 tube, 2 cone),
//!   `zdir`, `flag` (1 use max, 64 global coords, 256 use min, 512 use
//!   max radial, 1024 use min radial, 1<<14 apply to location, 1<<15
//!   rotation, 16 kill particles, 1<<16 culling, 1<<17 single sided),
//!   `f_strength`, `f_flow`, `f_noise`, `f_size`, `f_power`, `f_power_r`,
//!   `f_damp`, `maxdist`, `mindist`, `maxrad`, `minrad`, `seed`; and the
//!   collision part: `deflect`, `pdef_damp`, `pdef_rdamp`, `pdef_perm`,
//!   `pdef_frict`, `pdef_rfrict`, `pdef_stickness`, `absorption`,
//!   `pdef_sbdamp`, `pdef_sbift`, `pdef_sboft`, `pdef_cfrict`.
//! - `ClothModifierData.sim_parms` (`ClothSimSettings`) and `coll_parms`
//!   (`ClothCollSettings`); `ParticleSystemModifierData.psys` →
//!   `ParticleSystem` (`name`, `seed`, `part` → `ParticleSettings`);
//!   `FluidModifierData` (`type` 1 domain, 2 flow, 4 effector, and its
//!   `domain`, `flow`, `effector` settings); `Object.soft` (`SoftBody`).
//! - `EffectorWeights`: `weight[14]` indexed by field type (0 all),
//!   `global_gravity`, `group`.
use super::{BlendFile, View};
use crate::modifiers::Modifier;
use crate::physics::*;

fn fl(v: &View<'_, '_>, name: &str, into: &mut f32) {
    if let Some(x) = v.f32(name).filter(|x| x.is_finite()) {
        *into = x;
    }
}

fn int(v: &View<'_, '_>, name: &str, into: &mut i32) {
    if let Some(x) = v.num(name).filter(|x| x.is_finite()) {
        *into = x.clamp(i32::MIN as f64, i32::MAX as f64) as i32;
    }
}

fn flags(v: &View<'_, '_>, name: &str) -> i64 {
    v.num(name).unwrap_or(0.0) as i64
}

fn id_ref(f: &BlendFile<'_>, ptr: u64, names: &[&str]) -> String {
    f.view_ptr(ptr, names).map(|o| o.id_name()).unwrap_or_default()
}

fn effector_weights(f: &BlendFile<'_>, ptr: u64, w: &mut EffectorWeights) {
    let Some(v) = f.view_ptr(ptr, &["EffectorWeights"]) else { return };
    w.collection = id_ref(f, v.ptr("group"), &["Collection", "Group"]);
    fl(&v, "global_gravity", &mut w.gravity);
    let at = |i: usize, into: &mut f32| {
        if let Some(x) = v.num_at("weight", i).filter(|x| x.is_finite()) {
            *into = x as f32;
        }
    };
    at(0, &mut w.all);
    at(1, &mut w.force);
    at(2, &mut w.vortex);
    at(3, &mut w.magnetic);
    at(4, &mut w.wind);
    at(7, &mut w.harmonic);
    at(8, &mut w.charge);
    at(11, &mut w.turbulence);
    at(12, &mut w.drag);
}

fn point_cache(f: &BlendFile<'_>, ptr: u64, pc: &mut PointCacheSettings) {
    let Some(v) = f.view_ptr(ptr, &["PointCache"]) else { return };
    int(&v, "startframe", &mut pc.frame_start);
    int(&v, "endframe", &mut pc.frame_end);
    pc.use_disk_cache = flags(&v, "flag") & 64 != 0;
}

/// The scene's rigid body world and gravity.
pub(super) fn scene(f: &BlendFile<'_>, sc: &View<'_, '_>, ph: &mut ScenePhysics) {
    if let Some(p) = sc.sub("physics_settings") {
        if let Some(g) = p.vec3("gravity").filter(|g| g.to_array().iter().all(|x| x.is_finite())) {
            ph.gravity.gravity = g.to_array();
        }
        // PHYS_GLOBAL_GRAVITY = 1.
        if let Some(fl) = p.num("flag") {
            ph.gravity.use_gravity = fl as i64 & 1 != 0;
        }
    }
    let Some(rw) = f.view_ptr(sc.ptr("rigidbody_world"), &["RigidBodyWorld"]) else { return };
    let mut w = RigidBodyWorld::default();
    let fl_ = flags(&rw, "flag");
    w.enabled = fl_ & 1 == 0;
    w.use_split_impulse = fl_ & 4 != 0;
    // `steps_per_second` in the file (`substeps_per_frame` at runtime).
    if let Some(x) = rw.num_any(&["steps_per_second", "substeps_per_frame"]).filter(|x| x.is_finite()) {
        w.substeps_per_frame = x as i32;
    }
    int(&rw, "num_solver_iterations", &mut w.solver_iterations);
    fl(&rw, "time_scale", &mut w.time_scale);
    w.collection = id_ref(f, rw.ptr("group"), &["Collection", "Group"]);
    w.constraints = id_ref(f, rw.ptr("constraints"), &["Collection", "Group"]);
    effector_weights(f, rw.ptr("effector_weights"), &mut w.effector_weights);
    let pc = match rw.ptr("pointcache") {
        0 => f.view_ptr(rw.ptr("shared"), &["RigidBodyWorld_Shared"]).map_or(0, |s| s.ptr("pointcache")),
        p => p,
    };
    point_cache(f, pc, &mut w.point_cache);
    ph.rigidbody_world = Some(w);
}

/// Rigid body, rigid body constraint and force field of an object.
pub(super) fn object(f: &BlendFile<'_>, ob: &View<'_, '_>) -> ObjectPhysics {
    let mut ph = ObjectPhysics::default();
    if let Some(r) = f.view_ptr(ob.ptr("rigidbody_object"), &["RigidBodyOb"]) {
        let mut rb = RigidBody { kind: if flags(&r, "type") == 1 { RigidBodyType::Passive } else { RigidBodyType::Active }, ..Default::default() };
        rb.collision_shape = match flags(&r, "shape") {
            0 => CollisionShape::Box,
            1 => CollisionShape::Sphere,
            2 => CollisionShape::Capsule,
            3 => CollisionShape::Cylinder,
            4 => CollisionShape::Cone,
            6 => CollisionShape::Mesh,
            _ => CollisionShape::ConvexHull,
        };
        rb.mesh_source = match flags(&r, "mesh_source") {
            0 => MeshSource::Deform,
            1 => MeshSource::Final,
            _ => MeshSource::Base,
        };
        let fl_ = flags(&r, "flag");
        rb.kinematic = fl_ & 1 != 0;
        rb.use_deactivation = fl_ & 8 != 0;
        rb.use_start_deactivated = fl_ & 16 != 0;
        rb.enabled = fl_ & 32 == 0;
        rb.use_margin = fl_ & 64 != 0;
        fl(&r, "mass", &mut rb.mass);
        fl(&r, "friction", &mut rb.friction);
        fl(&r, "restitution", &mut rb.restitution);
        fl(&r, "margin", &mut rb.collision_margin);
        fl(&r, "lin_damping", &mut rb.linear_damping);
        fl(&r, "ang_damping", &mut rb.angular_damping);
        fl(&r, "lin_sleep_thresh", &mut rb.deactivate_linear_velocity);
        fl(&r, "ang_sleep_thresh", &mut rb.deactivate_angular_velocity);
        ph.rigid_body = Some(rb);
    }
    if let Some(c) = f.view_ptr(ob.ptr("rigidbody_constraint"), &["RigidBodyCon"]) {
        let mut rc = RigidBodyConstraint::default();
        rc.kind = match flags(&c, "type") {
            1 => ConstraintType::Fixed,
            2 => ConstraintType::Hinge,
            3 => ConstraintType::Slider,
            4 => ConstraintType::Piston,
            5 => ConstraintType::Generic,
            6 => ConstraintType::GenericSpring,
            7 => ConstraintType::Motor,
            _ => ConstraintType::Point,
        };
        rc.object1 = id_ref(f, c.ptr("ob1"), &["Object"]);
        rc.object2 = id_ref(f, c.ptr("ob2"), &["Object"]);
        let fl_ = flags(&c, "flag");
        rc.enabled = fl_ & 1 != 0;
        rc.disable_collisions = fl_ & 4 != 0;
        rc.use_breaking = fl_ & 8 != 0;
        rc.use_override_solver_iterations = fl_ & 16 != 0;
        fl(&c, "breaking_threshold", &mut rc.breaking_threshold);
        int(&c, "num_solver_iterations", &mut rc.solver_iterations);
        rc.spring_type = if flags(&c, "spring_type") == 0 { SpringType::Spring1 } else { SpringType::Spring2 };
        let axes = ["x", "y", "z"];
        for (k, ax) in rc.axes.iter_mut().enumerate() {
            let (kind, a) = if k < 3 { ("lin", axes[k]) } else { ("ang", axes[k - 3]) };
            ax.use_limit = fl_ & (1 << (5 + k)) != 0;
            ax.use_spring = fl_ & (1 << (11 + k)) != 0;
            fl(&c, &format!("limit_{kind}_{a}_lower"), &mut ax.lower);
            fl(&c, &format!("limit_{kind}_{a}_upper"), &mut ax.upper);
            let sfx = if k < 3 { a.to_string() } else { format!("ang_{a}") };
            fl(&c, &format!("spring_stiffness_{sfx}"), &mut ax.stiffness);
            fl(&c, &format!("spring_damping_{sfx}"), &mut ax.damping);
        }
        rc.use_motor_lin = fl_ & (1 << 17) != 0;
        rc.use_motor_ang = fl_ & (1 << 18) != 0;
        fl(&c, "motor_lin_target_velocity", &mut rc.motor_lin_target_velocity);
        fl(&c, "motor_ang_target_velocity", &mut rc.motor_ang_target_velocity);
        fl(&c, "motor_lin_max_impulse", &mut rc.motor_lin_max_impulse);
        fl(&c, "motor_ang_max_impulse", &mut rc.motor_ang_max_impulse);
        ph.rigid_body_constraint = Some(rc);
    }
    if let Some(pd) = f.view_ptr(ob.ptr("pd"), &["PartDeflect"]) {
        let kind = match flags(&pd, "forcefield") {
            1 => Some(FieldType::Force),
            2 => Some(FieldType::Vortex),
            3 => Some(FieldType::Magnetic),
            4 => Some(FieldType::Wind),
            7 => Some(FieldType::Harmonic),
            8 => Some(FieldType::Charge),
            11 => Some(FieldType::Turbulence),
            12 => Some(FieldType::Drag),
            _ => None,
        };
        if let Some(kind) = kind {
            let mut s = FieldSettings::new(kind);
            s.shape = if flags(&pd, "shape") == 1 { FieldShape::Plane } else { FieldShape::Point };
            s.falloff_type = match flags(&pd, "falloff") {
                1 => FalloffType::Tube,
                2 => FalloffType::Cone,
                _ => FalloffType::Sphere,
            };
            s.z_direction = match flags(&pd, "zdir") {
                1 => ZDirection::Positive,
                2 => ZDirection::Negative,
                _ => ZDirection::Both,
            };
            let fl_ = flags(&pd, "flag");
            s.use_max_distance = fl_ & 1 != 0;
            s.use_global_coords = fl_ & 64 != 0;
            s.use_min_distance = fl_ & 256 != 0;
            s.use_radial_max = fl_ & 512 != 0;
            s.use_radial_min = fl_ & 1024 != 0;
            // PFIELD_VISIBILITY: absorbed by collision objects.
            s.use_absorption = fl_ & (1 << 13) != 0;
            if pd.has("flag") {
                s.apply_to_location = fl_ & (1 << 14) != 0;
                s.apply_to_rotation = fl_ & (1 << 15) != 0;
            }
            if kind == FieldType::Drag {
                fl(&pd, "f_strength", &mut s.linear_drag);
                fl(&pd, "f_flow", &mut s.quadratic_drag);
            } else {
                fl(&pd, "f_strength", &mut s.strength);
                if kind == FieldType::Vortex {
                    fl(&pd, "f_flow", &mut s.inflow);
                } else {
                    fl(&pd, "f_flow", &mut s.flow);
                }
            }
            fl(&pd, "f_noise", &mut s.noise);
            int(&pd, "seed", &mut s.seed);
            if kind == FieldType::Harmonic {
                fl(&pd, "f_size", &mut s.rest_length);
                fl(&pd, "f_damp", &mut s.harmonic_damping);
            } else {
                fl(&pd, "f_size", &mut s.size);
            }
            fl(&pd, "f_power", &mut s.falloff_power);
            fl(&pd, "f_power_r", &mut s.radial_falloff);
            fl(&pd, "maxdist", &mut s.distance_max);
            fl(&pd, "mindist", &mut s.distance_min);
            fl(&pd, "maxrad", &mut s.radial_max);
            fl(&pd, "minrad", &mut s.radial_min);
            ph.field = Some(s);
        }
    }
    ph
}

/// Collision settings from the object's `PartDeflect`.
fn collision(f: &BlendFile<'_>, ob: &View<'_, '_>) -> CollisionSettings {
    let mut c = CollisionSettings::default();
    let Some(pd) = f.view_ptr(ob.ptr("pd"), &["PartDeflect"]) else { return c };
    if let Some(d) = pd.num("deflect") {
        c.use_collision = d != 0.0;
    }
    let fl_ = flags(&pd, "flag");
    c.use_particle_kill = fl_ & 16 != 0;
    if pd.has("flag") {
        // PFIELD_CLOTH_USE_CULLING, PFIELD_CLOTH_USE_NORMAL.
        c.use_culling = fl_ & (1 << 19) != 0;
        c.use_normal = fl_ & (1 << 20) != 0;
    }
    fl(&pd, "pdef_damp", &mut c.damping_factor);
    fl(&pd, "pdef_rdamp", &mut c.damping_random);
    fl(&pd, "pdef_perm", &mut c.permeability);
    fl(&pd, "pdef_frict", &mut c.friction_factor);
    fl(&pd, "pdef_rfrict", &mut c.friction_random);
    fl(&pd, "pdef_stickness", &mut c.stickiness);
    fl(&pd, "absorption", &mut c.absorption);
    fl(&pd, "pdef_sbdamp", &mut c.damping);
    fl(&pd, "pdef_sbift", &mut c.thickness_inner);
    fl(&pd, "pdef_sboft", &mut c.thickness_outer);
    fl(&pd, "pdef_cfrict", &mut c.cloth_friction);
    c
}

/// A 1-based vertex group index as a name.
fn group(groups: &[String], v: &View<'_, '_>, name: &str) -> Option<String> {
    let i = v.num(name)? as i64;
    (i >= 1).then(|| groups.get(i as usize - 1).cloned()).flatten()
}

fn cloth(f: &BlendFile<'_>, md: &View<'_, '_>, groups: &[String]) -> ClothSettings {
    let mut s = ClothSettings::default();
    if let Some(p) = f.view_ptr(md.ptr("sim_parms"), &["ClothSimSettings"]) {
        int(&p, "stepsPerFrame", &mut s.quality);
        fl(&p, "time_scale", &mut s.time_scale);
        fl(&p, "mass", &mut s.mass);
        fl(&p, "Cvi", &mut s.air_damping);
        s.bending_model = if flags(&p, "bending_model") == 1 { BendingModel::Linear } else { BendingModel::Angular };
        fl(&p, "tension", &mut s.tension_stiffness);
        fl(&p, "compression", &mut s.compression_stiffness);
        fl(&p, "shear", &mut s.shear_stiffness);
        fl(&p, "bending", &mut s.bending_stiffness);
        fl(&p, "tension_damp", &mut s.tension_damping);
        fl(&p, "compression_damp", &mut s.compression_damping);
        fl(&p, "shear_damp", &mut s.shear_damping);
        fl(&p, "bending_damping", &mut s.bending_damping);
        fl(&p, "goalspring", &mut s.pin_stiffness);
        fl(&p, "shrink_min", &mut s.shrink_min);
        fl(&p, "uniform_pressure_force", &mut s.uniform_pressure_force);
        fl(&p, "max_sewing", &mut s.sewing_force_max);
        let fl_ = flags(&p, "flags");
        s.use_dynamic_mesh = fl_ & (1 << 13) != 0;
        s.use_sewing_springs = fl_ & (1 << 14) != 0;
        s.use_pressure = fl_ & (1 << 17) != 0;
        s.vertex_group_mass = group(groups, &p, "vgroup_mass").unwrap_or_default();
        s.vertex_group_shrink = group(groups, &p, "vgroup_shrink").unwrap_or_default();
        effector_weights(f, p.ptr("effector_weights"), &mut s.effector_weights);
    }
    if let Some(c) = f.view_ptr(md.ptr("coll_parms"), &["ClothCollSettings"]) {
        let fl_ = flags(&c, "flags");
        s.use_collision = fl_ & 2 != 0;
        s.use_self_collision = fl_ & 4 != 0;
        int(&c, "loop_count", &mut s.collision_quality);
        fl(&c, "epsilon", &mut s.distance_min);
        fl(&c, "selfepsilon", &mut s.self_distance_min);
        fl(&c, "self_friction", &mut s.self_friction);
        fl(&c, "clamp", &mut s.impulse_clamp);
        s.collision_collection = id_ref(f, c.ptr("group"), &["Collection", "Group"]);
    }
    point_cache(f, md.ptr("point_cache"), &mut s.point_cache);
    s
}

fn softbody(f: &BlendFile<'_>, ob: &View<'_, '_>) -> SoftBodySettings {
    let mut s = SoftBodySettings::default();
    let Some(sb) = f.view_ptr(ob.ptr("soft"), &["SoftBody"]) else { return s };
    fl(&sb, "nodemass", &mut s.mass);
    fl(&sb, "mediafrict", &mut s.friction);
    fl(&sb, "physics_speed", &mut s.speed);
    fl(&sb, "goalspring", &mut s.goal_spring);
    fl(&sb, "goalfrict", &mut s.goal_friction);
    fl(&sb, "mingoal", &mut s.goal_min);
    fl(&sb, "maxgoal", &mut s.goal_max);
    fl(&sb, "defgoal", &mut s.goal_default);
    fl(&sb, "inspring", &mut s.pull);
    fl(&sb, "inpush", &mut s.push);
    fl(&sb, "infrict", &mut s.damping);
    fl(&sb, "colball", &mut s.ball_size);
    fl(&sb, "ballstiff", &mut s.ball_stiff);
    fl(&sb, "balldamp", &mut s.ball_damp);
    if let Some(p) = sb.num("plastic") {
        s.plastic = p as f32;
    }
    if let Some(b) = sb.num("secondspring") {
        s.bend = b as f32;
    }
    if let Some(l) = sb.num("springpreload") {
        s.spring_length = l as f32;
    }
    int(&sb, "minloops", &mut s.step_min);
    int(&sb, "maxloops", &mut s.step_max);
    s.collision_collection = id_ref(f, sb.ptr("collision_group"), &["Collection", "Group"]);
    s.vertex_group_goal = sb.string("namedVG_Softgoal").unwrap_or_default();
    s.vertex_group_mass = sb.string("namedVG_Mass").unwrap_or_default();
    // Object.softflag: 2 goal, 4 edges, 512 self collision.
    let sf = flags(ob, "softflag");
    if ob.has("softflag") {
        s.use_goal = sf & 2 != 0;
        s.use_edges = sf & 4 != 0;
        s.use_self_collision = sf & 512 != 0;
        s.use_face_collision = sf & (1 << 10) != 0;
        s.use_edge_collision = sf & (1 << 11) != 0;
    }
    effector_weights(f, sb.ptr("effector_weights"), &mut s.effector_weights);
    let pc = match sb.ptr("pointcache") {
        0 => f.view_ptr(sb.ptr("shared"), &["SoftBody_Shared"]).map_or(0, |x| x.ptr("pointcache")),
        p => p,
    };
    point_cache(f, pc, &mut s.point_cache);
    s
}

fn particle_system(f: &BlendFile<'_>, ps: &View<'_, '_>) -> ParticleSystem {
    let mut sys = ParticleSystem { name: ps.string("name").unwrap_or_else(|| "ParticleSystem".into()), ..Default::default() };
    int(ps, "seed", &mut sys.seed);
    point_cache(f, ps.ptr("pointcache"), &mut sys.point_cache);
    let Some(pa) = f.view_ptr(ps.ptr("part"), &["ParticleSettings"]) else { return sys };
    let hair = flags(&pa, "type") == 2;
    let mut s = if hair { ParticleSettings::hair() } else { ParticleSettings::default() };
    s.name = pa.id_name();
    int(&pa, "totpart", &mut s.count);
    fl(&pa, "sta", &mut s.frame_start);
    fl(&pa, "end", &mut s.frame_end);
    fl(&pa, "lifetime", &mut s.lifetime);
    fl(&pa, "randlife", &mut s.lifetime_random);
    int(&pa, "hair_step", &mut s.hair_step);
    s.emit_from = match flags(&pa, "from") {
        0 => EmitFrom::Vert,
        2 => EmitFrom::Volume,
        _ => EmitFrom::Face,
    };
    s.distribution = match flags(&pa, "distr") {
        1 => Distribution::Random,
        2 => Distribution::Grid,
        _ => Distribution::Jittered,
    };
    fl(&pa, "jitfac", &mut s.jitter_factor);
    int(&pa, "userjit", &mut s.userjit);
    int(&pa, "grid_res", &mut s.grid_resolution);
    fl(&pa, "normfac", &mut s.normal_factor);
    if hair {
        fl(&pa, "normfac", &mut s.hair_length);
    }
    fl(&pa, "tanfac", &mut s.tangent_factor);
    fl(&pa, "tanphase", &mut s.tangent_phase);
    fl(&pa, "obfac", &mut s.object_factor);
    fl(&pa, "randfac", &mut s.factor_random);
    if let Some(v) = pa.vec::<3>("ob_vel") {
        s.object_align_factor = v;
    }
    s.physics_type = if flags(&pa, "phystype") == 0 { ParticlePhysics::None } else { ParticlePhysics::Newton };
    s.integrator = match flags(&pa, "integrator") {
        0 => Integrator::Euler,
        2 => Integrator::Rk4,
        3 => Integrator::Verlet,
        _ => Integrator::Midpoint,
    };
    fl(&pa, "mass", &mut s.mass);
    fl(&pa, "size", &mut s.particle_size);
    fl(&pa, "randsize", &mut s.size_random);
    fl(&pa, "dragfac", &mut s.drag_factor);
    fl(&pa, "brownfac", &mut s.brownian_factor);
    fl(&pa, "dampfac", &mut s.damping);
    int(&pa, "subframes", &mut s.subframes);
    // ren_as / draw_as: 0 none, 1 dot (halo), 2 circle, 3 cross, 4 axis,
    // 5 line, 6 path, 7 object, 8 collection.
    s.render_type = match flags(&pa, "ren_as") {
        0 => RenderType::None,
        5 => RenderType::Line,
        6 => RenderType::Path,
        7 => RenderType::Object,
        8 => RenderType::Collection,
        _ if hair => RenderType::Path,
        _ => RenderType::Halo,
    };
    s.display_method = match flags(&pa, "draw_as") {
        0 => DisplayMethod::None,
        1 => DisplayMethod::Point,
        2 => DisplayMethod::Circle,
        3 => DisplayMethod::Cross,
        4 => DisplayMethod::Axis,
        _ => DisplayMethod::Rendered,
    };
    s.instance_object = id_ref(f, pa.ptr("instance_object"), &["Object"]);
    if s.instance_object.is_empty() {
        s.instance_object = id_ref(f, pa.ptr("dup_ob"), &["Object"]);
    }
    s.instance_collection = id_ref(f, pa.ptr("instance_collection"), &["Collection", "Group"]);
    if s.instance_collection.is_empty() {
        s.instance_collection = id_ref(f, pa.ptr("dup_group"), &["Collection", "Group"]);
    }
    int(&pa, "omat", &mut s.material_slot);
    int(&pa, "disp", &mut s.display_percentage);
    fl(&pa, "draw_size", &mut s.display_size);
    int(&pa, "draw_step", &mut s.display_step);
    int(&pa, "ren_step", &mut s.render_step);
    fl(&pa, "rad_root", &mut s.root_radius);
    fl(&pa, "rad_tip", &mut s.tip_radius);
    fl(&pa, "rad_scale", &mut s.radius_scale);
    s.child_type = match flags(&pa, "childtype") {
        1 => ChildType::Simple,
        2 => ChildType::Interpolated,
        _ => ChildType::None,
    };
    if let Some(n) = pa.num_any(&["child_nbr", "child_percent"]) {
        s.child_nbr = n as i32;
    }
    if let Some(n) = pa.num_any(&["ren_child_nbr", "child_render_percent"]) {
        s.rendered_child_count = n as i32;
    }
    fl(&pa, "childrad", &mut s.child_radius);
    fl(&pa, "childflat", &mut s.child_roundness);
    fl(&pa, "clumpfac", &mut s.clump_factor);
    fl(&pa, "rough1", &mut s.roughness_1);
    fl(&pa, "clength", &mut s.child_length);
    if let Some(v) = pa.vec::<2>("draw_line") {
        (s.line_length_tail, s.line_length_head) = (v[0], v[1]);
    }
    // ParticleSettings.flag: PART_UNBORN 1 << 5, PART_DIED 1 << 6,
    // PART_TRAND 1 << 7, PART_EDISTR 1 << 8.
    if pa.has("flag") {
        let pf = flags(&pa, "flag");
        s.show_unborn = pf & (1 << 5) != 0;
        s.use_dead = pf & (1 << 6) != 0;
        s.use_emit_random = pf & (1 << 7) != 0;
        s.use_even_distribution = pf & (1 << 8) != 0;
    }
    // ParticleSettings.draw: PART_DRAW_ROTATE_OB 1 << 7, PART_DRAW_RAND_GR
    // 1 << 10, PART_DRAW_NO_SCALE_OB 1 << 16.
    if pa.has("draw") {
        let d = flags(&pa, "draw");
        s.use_rotation_instance = d & (1 << 7) != 0;
        s.use_collection_pick_random = d & (1 << 10) != 0;
        s.use_scale_instance = d & (1 << 16) == 0;
    }
    effector_weights(f, pa.ptr("effector_weights"), &mut s.effector_weights);
    sys.settings = s;
    sys
}

fn fluid(f: &BlendFile<'_>, md: &View<'_, '_>) -> Option<FluidSettings> {
    let ty = flags(md, "type");
    let mut s = match ty {
        1 => FluidSettings::new(FluidType::Domain),
        2 => FluidSettings::new(FluidType::Flow),
        4 => FluidSettings::new(FluidType::Effector),
        _ => return None,
    };
    if let Some(d) = f.view_ptr(md.ptr("domain"), &["FluidDomainSettings"]) {
        s.domain_type = if flags(&d, "type") == 1 { DomainType::Liquid } else { DomainType::Gas };
        int(&d, "maxres", &mut s.resolution_max);
        fl(&d, "time_scale", &mut s.time_scale);
        fl(&d, "cfl_condition", &mut s.cfl_condition);
        // `timesteps_maximum`/`minimum` in 4.2 DNA.
        if let Some(x) = d.num_any(&["timesteps_maximum", "timesteps_max"]) {
            s.timesteps_max = x as i32;
        }
        if let Some(x) = d.num_any(&["timesteps_minimum", "timesteps_min"]) {
            s.timesteps_min = x as i32;
        }
        fl(&d, "alpha", &mut s.alpha);
        fl(&d, "beta", &mut s.beta);
        fl(&d, "vorticity", &mut s.vorticity);
        if let Some(x) = d.num("diss_speed") {
            s.dissolve_speed = x as f32;
        }
        fl(&d, "burning_rate", &mut s.burning_rate);
        fl(&d, "flame_smoke", &mut s.flame_smoke);
        fl(&d, "flame_vorticity", &mut s.flame_vorticity);
        fl(&d, "flame_max_temp", &mut s.flame_max_temp);
        int(&d, "cache_frame_start", &mut s.cache_frame_start);
        int(&d, "cache_frame_end", &mut s.cache_frame_end);
        effector_weights(f, d.ptr("effector_weights"), &mut s.effector_weights);
        for (k, v) in [
            ("flip_ratio", &mut s.flip_ratio),
            ("particle_radius", &mut s.particle_radius),
            ("particle_randomness", &mut s.particle_randomness),
            ("particle_band_width", &mut s.particle_band_width),
            ("fractions_distance", &mut s.fractions_distance),
            ("fractions_threshold", &mut s.fractions_threshold),
            ("mesh_particle_radius", &mut s.mesh_particle_radius),
            ("mesh_concave_upper", &mut s.mesh_concave_upper),
            ("mesh_concave_lower", &mut s.mesh_concave_lower),
            ("viscosity_base", &mut s.viscosity_base),
            ("surface_tension", &mut s.surface_tension),
            ("viscosity_value", &mut s.viscosity_value),
            ("clipping", &mut s.clipping),
        ] {
            fl(&d, k, v);
        }
        for (k, v) in [
            ("sys_particle_maximum", &mut s.sys_particle_maximum),
            ("particle_number", &mut s.particle_number),
            ("particle_maximum", &mut s.particle_max),
            ("particle_minimum", &mut s.particle_min),
            ("mesh_scale", &mut s.mesh_scale),
            ("mesh_smoothen_pos", &mut s.mesh_smoothen_pos),
            ("mesh_smoothen_neg", &mut s.mesh_smoothen_neg),
            ("viscosity_exponent", &mut s.viscosity_exponent),
            ("cache_frame_offset", &mut s.cache_frame_offset),
        ] {
            int(&d, k, v);
        }
        if let Some(g) = d.vec::<3>("gravity") {
            s.gravity = g;
        }
        s.simulation_method = if flags(&d, "simulation_method") == 1 { SimulationMethod::Apic } else { SimulationMethod::Flip };
        s.mesh_generator = if flags(&d, "mesh_generator") == 1 { MeshGenerator::Union } else { MeshGenerator::Improved };
        s.cache_type = match flags(&d, "cache_type") {
            1 => FluidCacheType::Modular,
            2 => FluidCacheType::All,
            _ => FluidCacheType::Replay,
        };
        if let Some(p) = d.num("particle_type") {
            s.use_flip_particles = p as i64 & 1 != 0;
        }
        if let Some(b) = d.num("border_collisions").map(|b| b as i64) {
            s.use_collision_border_front = b & 2 != 0;
            s.use_collision_border_back = b & 4 != 0;
            s.use_collision_border_right = b & 8 != 0;
            s.use_collision_border_left = b & 16 != 0;
            s.use_collision_border_top = b & 32 != 0;
            s.use_collision_border_bottom = b & 64 != 0;
        }
        if let Some(b) = d.num("flags").map(|b| b as i64) {
            let on = |bit: i64| b & bit != 0;
            s.use_dissolve_smoke = on(1 << 2);
            s.use_dissolve_smoke_log = on(1 << 3);
            s.use_speed_vectors = on(1 << 6);
            s.use_fractions = on(1 << 8);
            s.delete_in_obstacle = on(1 << 9);
            s.use_diffusion = on(1 << 10);
            s.use_mesh = on(1 << 11);
            s.cache_resumable = on(1 << 12);
            s.use_adaptive_timesteps = on(1 << 13);
            s.use_viscosity = on(1 << 14);
        }
    }
    if let Some(fw) = f.view_ptr(md.ptr("flow"), &["FluidFlowSettings"]) {
        s.flow_type = match flags(&fw, "type") {
            2 => FlowType::Fire,
            3 => FlowType::Both,
            4 => FlowType::Liquid,
            _ => FlowType::Smoke,
        };
        s.flow_behavior = match flags(&fw, "behavior") {
            0 => FlowBehavior::Inflow,
            1 => FlowBehavior::Outflow,
            _ => FlowBehavior::Geometry,
        };
        fl(&fw, "density", &mut s.density);
        if let Some(c) = fw.vec::<3>("color") {
            s.smoke_color = c;
        }
        fl(&fw, "fuel_amount", &mut s.fuel_amount);
        fl(&fw, "temperature", &mut s.temperature);
        fl(&fw, "volume_density", &mut s.volume_density);
        fl(&fw, "surface_distance", &mut s.surface_distance);
        fl(&fw, "vel_multi", &mut s.velocity_factor);
        fl(&fw, "vel_normal", &mut s.velocity_normal);
        if let Some(v) = fw.vec::<3>("vel_coord") {
            s.velocity_coord = v;
        }
        int(&fw, "subframes", &mut s.subframes);
        let b = flags(&fw, "flags");
        s.use_absolute = b & 2 != 0;
        s.use_initial_velocity = b & 4 != 0;
        s.use_plane_init = b & 32 != 0;
        s.use_inflow = b & 64 != 0;
    }
    if let Some(e) = f.view_ptr(md.ptr("effector"), &["FluidEffectorSettings"]) {
        s.effector_type = if flags(&e, "type") == 1 { EffectorType::Guide } else { EffectorType::Collision };
        fl(&e, "surface_distance", &mut s.effector_surface_distance);
        int(&e, "subframes", &mut s.subframes);
        let b = flags(&e, "flags");
        s.use_effector = b & 2 != 0;
        s.use_plane_init = b & 4 != 0;
    }
    Some(s)
}

/// A physics modifier block, read as its concrete DNA struct.
pub(super) fn modifier(f: &BlendFile<'_>, ob: &View<'_, '_>, md: &View<'_, '_>, groups: &[String]) -> Option<Modifier> {
    let p = match md.struct_name() {
        "ClothModifierData" => PhysicsModifier::Cloth(Box::new(cloth(f, md, groups))),
        "SoftbodyModifierData" => PhysicsModifier::SoftBody(Box::new(softbody(f, ob))),
        "CollisionModifierData" => PhysicsModifier::Collision(collision(f, ob)),
        "ParticleSystemModifierData" => {
            let mut ps = particle_system(f, &f.view_ptr(md.ptr("psys"), &["ParticleSystem"])?);
            // Show Emitter in renders: Object.duplicator_visibility_flag
            // OB_DUPLI_FLAG_RENDER.
            if ob.has("duplicator_visibility_flag") {
                ps.settings.use_render_emitter = flags(ob, "duplicator_visibility_flag") & 2 != 0;
            }
            PhysicsModifier::ParticleSystem(Box::new(ps))
        }
        "FluidModifierData" => PhysicsModifier::Fluid(Box::new(fluid(f, md)?)),
        _ => return None,
    };
    Some(Modifier::Physics(p))
}
