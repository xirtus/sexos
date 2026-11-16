// SPDX-License-Identifier: GPL-3.0-only
//! Physics for the `.blend` writer, the inverse of `blend/physics.rs`: the
//! scene's gravity and rigid body world, objects' rigid bodies, rigid body
//! constraints, force fields and collision settings (`PartDeflect`), soft
//! bodies, and the Cloth, Softbody, Collision, Particle System and Fluid
//! modifiers with their settings, point caches and effector weights. Member
//! names and values follow Blender 4.2's `DNA_rigidbody_types.h`,
//! `DNA_object_force_types.h`, `DNA_cloth_types.h`, `DNA_particle_types.h`,
//! `DNA_fluid_types.h` and `DNA_pointcache_types.h` (GPL-2+). Caches are
//! written empty (no baked frames).
use super::export::Ids;
use super::write::{Rec, W};
use super::write_rig::{inv, StackExtras};
use crate::physics::*;
use crate::scene::Scene;

fn bits(on: &[(bool, i64)]) -> i64 {
    on.iter().filter(|(b, _)| *b).fold(0, |a, (_, m)| a | m)
}

/// A 1-based vertex group index (0: none).
fn group_index(groups: &[String], name: &str) -> i64 {
    if name.is_empty() {
        return 0;
    }
    groups.iter().position(|g| g == name).map_or(0, |i| i as i64 + 1)
}

impl W {
    /// An `EffectorWeights` block.
    fn effector_weights(&mut self, w: &EffectorWeights, ids: &Ids) -> u64 {
        let mut r = self.rec("EffectorWeights");
        self.p(&mut r, "group", ids.collection(&w.collection));
        let mut weight = [1.0f32; 14];
        for (i, v) in [(0, w.all), (1, w.force), (2, w.vortex), (3, w.magnetic), (4, w.wind), (7, w.harmonic), (8, w.charge), (11, w.turbulence), (12, w.drag)] {
            weight[i] = v;
        }
        self.f(&mut r, "weight", &weight);
        self.f(&mut r, "global_gravity", &[w.gravity]);
        self.i(&mut r, "flag", 1); // EFF_WEIGHT_DO_HAIR
        self.data(&[r])
    }

    /// A `PointCache` (BKE_ptcache_add's values) linked as the only entry
    /// of the `ptcaches` list at `list` in `owner`; returns its address.
    fn point_cache(&mut self, pc: &PointCacheSettings, owner: &mut Rec, list: &str) -> u64 {
        let mut r = self.rec("PointCache");
        self.i(&mut r, "startframe", pc.frame_start as i64);
        self.i(&mut r, "endframe", pc.frame_end as i64);
        self.i(&mut r, "step", 1);
        self.i(&mut r, "index", -1);
        self.i(&mut r, "flag", bits(&[(pc.use_disk_cache, 1 << 6)]));
        let a = self.data(&[r]);
        self.listbase(owner, list, &[a]);
        a
    }

    // ------------------------------------------------------------- Scene

    /// Gravity on the scene record and, when there is one, the rigid body
    /// world (written as the scene's data).
    pub(super) fn scene_physics(&mut self, sc: &mut Rec, ph: &ScenePhysics, ids: &Ids) {
        self.f(sc, "physics_settings.gravity", &ph.gravity.gravity);
        let f = self.get(sc, "physics_settings.flag") & !1;
        self.i(sc, "physics_settings.flag", f | ph.gravity.use_gravity as i64); // PHYS_GLOBAL_GRAVITY
        let Some(w) = &ph.rigidbody_world else { return };
        let mut r = self.rec("RigidBodyWorld");
        self.p(&mut r, "group", ids.collection(&w.collection));
        self.p(&mut r, "constraints", ids.collection(&w.constraints));
        self.i(&mut r, "flag", bits(&[(!w.enabled, 1), (w.use_split_impulse, 4)]));
        self.i(&mut r, "steps_per_second", w.substeps_per_frame.clamp(0, 32767) as i64);
        self.i(&mut r, "num_solver_iterations", w.solver_iterations.clamp(0, 32767) as i64);
        self.f(&mut r, "time_scale", &[w.time_scale]);
        self.f(&mut r, "ltime", &[w.point_cache.frame_start as f32]);
        let ew = self.effector_weights(&w.effector_weights, ids);
        self.p(&mut r, "effector_weights", ew);
        let mut sh = self.rec("RigidBodyWorld_Shared");
        let pc = self.point_cache(&w.point_cache, &mut sh, "ptcaches");
        self.p(&mut sh, "pointcache", pc);
        let sha = self.data(&[sh]);
        self.p(&mut r, "shared", sha);
        let a = self.data(&[r]);
        self.p(sc, "rigidbody_world", a);
    }

    // ------------------------------------------------------------ Object

    /// Rigid body, rigid body constraint and the `PartDeflect` (force field
    /// plus the Collision modifier's settings) of an object record.
    pub(super) fn object_physics(&mut self, ob: &mut Rec, ph: &ObjectPhysics, collision: Option<&CollisionSettings>, ids: &Ids) {
        if let Some(rb) = &ph.rigid_body {
            let mut r = self.rec("RigidBodyOb");
            self.i(&mut r, "type", (rb.kind == RigidBodyType::Passive) as i64);
            self.i(&mut r, "shape", match rb.collision_shape {
                CollisionShape::Box => 0,
                CollisionShape::Sphere => 1,
                CollisionShape::Capsule => 2,
                CollisionShape::Cylinder => 3,
                CollisionShape::Cone => 4,
                CollisionShape::ConvexHull => 5,
                CollisionShape::Mesh => 6,
            });
            self.i(&mut r, "mesh_source", match rb.mesh_source {
                MeshSource::Deform => 0,
                MeshSource::Final => 1,
                MeshSource::Base => 2,
            });
            // RBO_FLAG_NEEDS_VALIDATE | NEEDS_RESHAPE so Blender builds the body.
            self.i(&mut r, "flag", 2 | 4 | bits(&[(rb.kinematic, 1), (rb.use_deactivation, 8), (rb.use_start_deactivated, 16), (!rb.enabled, 32), (rb.use_margin, 64)]));
            self.i(&mut r, "col_groups", 1);
            for (k, v) in [("mass", rb.mass), ("friction", rb.friction), ("restitution", rb.restitution), ("margin", rb.collision_margin), ("lin_damping", rb.linear_damping), ("ang_damping", rb.angular_damping), ("lin_sleep_thresh", rb.deactivate_linear_velocity), ("ang_sleep_thresh", rb.deactivate_angular_velocity)] {
                self.f(&mut r, k, &[v]);
            }
            self.f(&mut r, "orn", &[1.0, 0.0, 0.0, 0.0]);
            let a = self.data(&[r]);
            self.p(ob, "rigidbody_object", a);
        }
        if let Some(c) = &ph.rigid_body_constraint {
            let mut r = self.rec("RigidBodyCon");
            self.i(&mut r, "type", inv(constraint_type, c.kind));
            self.p(&mut r, "ob1", ids.object(&c.object1));
            self.p(&mut r, "ob2", ids.object(&c.object2));
            let mut fl = bits(&[(c.enabled, 1), (c.disable_collisions, 4), (c.use_breaking, 8), (c.use_override_solver_iterations, 16), (c.use_motor_lin, 1 << 17), (c.use_motor_ang, 1 << 18)]);
            let axes = ["x", "y", "z"];
            for (k, ax) in c.axes.iter().enumerate() {
                fl |= bits(&[(ax.use_limit, 1 << (5 + k)), (ax.use_spring, 1 << (11 + k))]);
                let (kind, a) = if k < 3 { ("lin", axes[k]) } else { ("ang", axes[k - 3]) };
                self.f(&mut r, &format!("limit_{kind}_{a}_lower"), &[ax.lower]);
                self.f(&mut r, &format!("limit_{kind}_{a}_upper"), &[ax.upper]);
                let sfx = if k < 3 { a.to_string() } else { format!("ang_{a}") };
                self.f(&mut r, &format!("spring_stiffness_{sfx}"), &[ax.stiffness]);
                self.f(&mut r, &format!("spring_damping_{sfx}"), &[ax.damping]);
            }
            self.i(&mut r, "flag", fl | 2); // RBC_FLAG_NEEDS_VALIDATE
            self.f(&mut r, "breaking_threshold", &[c.breaking_threshold]);
            self.i(&mut r, "num_solver_iterations", c.solver_iterations.clamp(0, 32767) as i64);
            self.i(&mut r, "spring_type", (c.spring_type == SpringType::Spring2) as i64);
            for (k, v) in [("motor_lin_target_velocity", c.motor_lin_target_velocity), ("motor_ang_target_velocity", c.motor_ang_target_velocity), ("motor_lin_max_impulse", c.motor_lin_max_impulse), ("motor_ang_max_impulse", c.motor_ang_max_impulse)] {
                self.f(&mut r, k, &[v]);
            }
            let a = self.data(&[r]);
            self.p(ob, "rigidbody_constraint", a);
        }
        if ph.field.is_none() && collision.is_none() {
            return;
        }
        // BKE_partdeflect_new's values, then the field and collision settings.
        let mut pd = self.rec("PartDeflect");
        for (k, v) in [("pdef_sbdamp", 0.1), ("pdef_sbift", 0.2), ("pdef_sboft", 0.02), ("pdef_cfrict", 5.0), ("f_strength", 1.0), ("f_power", 1.0), ("f_size", 0.0), ("f_damp", 1.0), ("maxdist", 0.0), ("mindist", 0.0), ("f_flow", 0.0), ("f_noise", 0.0), ("pdef_frict", 0.5), ("pdef_damp", 0.1)] {
            self.f(&mut pd, k, &[v]);
        }
        self.i(&mut pd, "seed", 0);
        let mut flag = 0;
        if let Some(s) = &ph.field {
            self.i(&mut pd, "forcefield", match s.kind {
                FieldType::Force => 1,
                FieldType::Vortex => 2,
                FieldType::Magnetic => 3,
                FieldType::Wind => 4,
                FieldType::Harmonic => 7,
                FieldType::Charge => 8,
                FieldType::Turbulence => 11,
                FieldType::Drag => 12,
            });
            self.i(&mut pd, "shape", (s.shape == FieldShape::Plane) as i64);
            self.i(&mut pd, "falloff", match s.falloff_type {
                FalloffType::Sphere => 0,
                FalloffType::Tube => 1,
                FalloffType::Cone => 2,
            });
            self.i(&mut pd, "zdir", match s.z_direction {
                ZDirection::Both => 0,
                ZDirection::Positive => 1,
                ZDirection::Negative => 2,
            });
            flag |= bits(&[(s.use_max_distance, 1), (s.use_global_coords, 64), (s.use_min_distance, 256), (s.use_radial_max, 512), (s.use_radial_min, 1024), (s.use_absorption, 1 << 13), (s.apply_to_location, 1 << 14), (s.apply_to_rotation, 1 << 15)]);
            let (strength, flow) = match s.kind {
                FieldType::Drag => (s.linear_drag, s.quadratic_drag),
                FieldType::Vortex => (s.strength, s.inflow),
                _ => (s.strength, s.flow),
            };
            self.f(&mut pd, "f_strength", &[strength]);
            self.f(&mut pd, "f_flow", &[flow]);
            self.f(&mut pd, "f_noise", &[s.noise]);
            self.i(&mut pd, "seed", s.seed as i64);
            if s.kind == FieldType::Harmonic {
                self.f(&mut pd, "f_size", &[s.rest_length]);
                self.f(&mut pd, "f_damp", &[s.harmonic_damping]);
            } else {
                self.f(&mut pd, "f_size", &[s.size]);
            }
            for (k, v) in [("f_power", s.falloff_power), ("f_power_r", s.radial_falloff), ("maxdist", s.distance_max), ("mindist", s.distance_min), ("maxrad", s.radial_max), ("minrad", s.radial_min)] {
                self.f(&mut pd, k, &[v]);
            }
        }
        if let Some(c) = collision {
            self.i(&mut pd, "deflect", c.use_collision as i64);
            // PDEFLE_KILL_PART, PFIELD_CLOTH_USE_CULLING, PFIELD_CLOTH_USE_NORMAL.
            flag |= bits(&[(c.use_particle_kill, 16), (c.use_culling, 1 << 19), (c.use_normal, 1 << 20)]);
            for (k, v) in [("pdef_damp", c.damping_factor), ("pdef_rdamp", c.damping_random), ("pdef_perm", c.permeability), ("pdef_frict", c.friction_factor), ("pdef_rfrict", c.friction_random), ("pdef_stickness", c.stickiness), ("absorption", c.absorption), ("pdef_sbdamp", c.damping), ("pdef_sbift", c.thickness_inner), ("pdef_sboft", c.thickness_outer), ("pdef_cfrict", c.cloth_friction)] {
                self.f(&mut pd, k, &[v]);
            }
        }
        self.i(&mut pd, "flag", flag);
        let a = self.data(&[pd]);
        self.p(ob, "pd", a);
    }

    /// `Object.soft` and the soft body flags in `Object.softflag`.
    pub(super) fn soft_body(&mut self, ob: &mut Rec, s: &SoftBodySettings, ids: &Ids) {
        let mut r = self.rec("SoftBody");
        // sbNew's values for what Polyamour does not hold.
        for (k, v) in [("grav", 9.8), ("rklimit", 0.1), ("shearstiff", 1.0), ("physics_speed", 1.0)] {
            self.f(&mut r, k, &[v]);
        }
        self.i(&mut r, "sbc_mode", 1);
        self.i(&mut r, "choke", 3);
        for (k, v) in [
            ("nodemass", s.mass),
            ("mediafrict", s.friction),
            ("physics_speed", s.speed),
            ("goalspring", s.goal_spring),
            ("goalfrict", s.goal_friction),
            ("mingoal", s.goal_min),
            ("maxgoal", s.goal_max),
            ("defgoal", s.goal_default),
            ("inspring", s.pull),
            ("inpush", s.push),
            ("infrict", s.damping),
            ("colball", s.ball_size),
            ("ballstiff", s.ball_stiff),
            ("balldamp", s.ball_damp),
            ("secondspring", s.bend),
        ] {
            self.f(&mut r, k, &[v]);
        }
        self.i(&mut r, "plastic", s.plastic.round() as i64);
        self.i(&mut r, "springpreload", s.spring_length.round() as i64);
        self.i(&mut r, "minloops", s.step_min as i64);
        self.i(&mut r, "maxloops", s.step_max as i64);
        self.s(&mut r, "namedVG_Softgoal", &s.vertex_group_goal);
        self.s(&mut r, "namedVG_Mass", &s.vertex_group_mass);
        self.p(&mut r, "collision_group", ids.collection(&s.collision_collection));
        let ew = self.effector_weights(&s.effector_weights, ids);
        self.p(&mut r, "effector_weights", ew);
        let mut sh = self.rec("SoftBody_Shared");
        let pc = self.point_cache(&s.point_cache, &mut sh, "ptcaches");
        self.p(&mut sh, "pointcache", pc);
        let sha = self.data(&[sh]);
        self.p(&mut r, "shared", sha);
        let a = self.data(&[r]);
        self.p(ob, "soft", a);
        let f = self.get(ob, "softflag") & !(2 | 4 | 512 | 1 << 10 | 1 << 11);
        self.i(ob, "softflag", f | bits(&[(s.use_goal, 2), (s.use_edges, 4), (s.use_self_collision, 512), (s.use_face_collision, 1 << 10), (s.use_edge_collision, 1 << 11)]));
    }

    // ---------------------------------------------------------- Modifiers

    /// Fill a physics modifier's struct.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn physics_modifier(&mut self, r: &mut Rec, p: &PhysicsModifier, _scene: &Scene, oi: usize, k: usize, ids: &Ids, groups: &[String], extras: &mut StackExtras) {
        match p {
            PhysicsModifier::Cloth(s) => self.cloth(r, s, ids, groups),
            // Their settings are on the object (`soft`, `pd`).
            PhysicsModifier::SoftBody(_) | PhysicsModifier::Collision(_) => {}
            PhysicsModifier::ParticleSystem(ps) => {
                let part = ids.particles.get(&(oi, k)).copied().unwrap_or(0);
                let mut sys = self.rec("ParticleSystem");
                self.s(&mut sys, "name", &ps.name);
                self.i(&mut sys, "seed", ps.seed as i64);
                self.i(&mut sys, "child_seed", ps.seed as i64);
                self.p(&mut sys, "part", part);
                self.f(&mut sys, "lattice_strength", &[1.0]);
                let pc = self.point_cache(&ps.point_cache, &mut sys, "ptcaches");
                self.p(&mut sys, "pointcache", pc);
                let a = self.addr();
                extras.particle_systems.push(a);
                extras.pending_psys.push((a, sys));
                self.p(r, "psys", a);
            }
            PhysicsModifier::Fluid(s) => self.fluid(r, s, ids),
        }
    }

    fn cloth(&mut self, r: &mut Rec, s: &ClothSettings, ids: &Ids, groups: &[String]) {
        let mut p = self.rec("ClothSimSettings");
        self.i(&mut p, "stepsPerFrame", s.quality as i64);
        for (k, v) in [
            ("time_scale", s.time_scale),
            ("mass", s.mass),
            ("Cvi", s.air_damping),
            ("tension", s.tension_stiffness),
            ("compression", s.compression_stiffness),
            ("shear", s.shear_stiffness),
            ("bending", s.bending_stiffness),
            ("tension_damp", s.tension_damping),
            ("compression_damp", s.compression_damping),
            ("shear_damp", s.shear_damping),
            ("bending_damping", s.bending_damping),
            ("goalspring", s.pin_stiffness),
            ("shrink_min", s.shrink_min),
            ("uniform_pressure_force", s.uniform_pressure_force),
            ("max_sewing", s.sewing_force_max),
        ] {
            self.f(&mut p, k, &[v]);
        }
        self.i(&mut p, "bending_model", (s.bending_model == BendingModel::Linear) as i64);
        let f = self.get(&p, "flags") & !(1 << 13 | 1 << 14 | 1 << 17);
        self.i(&mut p, "flags", f | bits(&[(s.use_dynamic_mesh, 1 << 13), (s.use_sewing_springs, 1 << 14), (s.use_pressure, 1 << 17)]));
        self.i(&mut p, "vgroup_mass", group_index(groups, &s.vertex_group_mass));
        self.i(&mut p, "vgroup_shrink", group_index(groups, &s.vertex_group_shrink));
        let ew = self.effector_weights(&s.effector_weights, ids);
        self.p(&mut p, "effector_weights", ew);
        let pa = self.data(&[p]);
        let mut c = self.rec("ClothCollSettings");
        let f = self.get(&c, "flags") & !(2 | 4);
        self.i(&mut c, "flags", f | bits(&[(s.use_collision, 2), (s.use_self_collision, 4)]));
        self.i(&mut c, "loop_count", s.collision_quality as i64);
        for (k, v) in [("epsilon", s.distance_min), ("selfepsilon", s.self_distance_min), ("self_friction", s.self_friction), ("clamp", s.impulse_clamp)] {
            self.f(&mut c, k, &[v]);
        }
        self.p(&mut c, "group", ids.collection(&s.collision_collection));
        let ca = self.data(&[c]);
        self.p(r, "sim_parms", pa);
        self.p(r, "coll_parms", ca);
        let pc = self.point_cache(&s.point_cache, r, "ptcaches");
        self.p(r, "point_cache", pc);
    }

    fn fluid(&mut self, r: &mut Rec, s: &FluidSettings, ids: &Ids) {
        match s.fluid_type {
            FluidType::Domain => {
                self.i(r, "type", 1);
                let mut d = self.rec("FluidDomainSettings");
                self.i(&mut d, "type", (s.domain_type == DomainType::Liquid) as i64);
                self.i(&mut d, "maxres", s.resolution_max as i64);
                for (k, v) in [
                    ("time_scale", s.time_scale),
                    ("cfl_condition", s.cfl_condition),
                    ("alpha", s.alpha),
                    ("beta", s.beta),
                    ("vorticity", s.vorticity),
                    ("burning_rate", s.burning_rate),
                    ("flame_smoke", s.flame_smoke),
                    ("flame_vorticity", s.flame_vorticity),
                    ("flame_max_temp", s.flame_max_temp),
                ] {
                    self.f(&mut d, k, &[v]);
                }
                self.i(&mut d, "timesteps_maximum", s.timesteps_max as i64);
                self.i(&mut d, "timesteps_minimum", s.timesteps_min as i64);
                self.i(&mut d, "diss_speed", s.dissolve_speed.round() as i64);
                self.i(&mut d, "cache_frame_start", s.cache_frame_start as i64);
                self.i(&mut d, "cache_frame_end", s.cache_frame_end as i64);
                let ew = self.effector_weights(&s.effector_weights, ids);
                self.p(&mut d, "effector_weights", ew);
                let pc = self.point_cache(&PointCacheSettings { frame_start: s.cache_frame_start, frame_end: s.cache_frame_end, use_disk_cache: true }, &mut d, "ptcaches");
                self.p(&mut d, "point_cache", pc);
                let a = self.data(&[d]);
                self.p(r, "domain", a);
            }
            FluidType::Flow => {
                self.i(r, "type", 2);
                let mut fw = self.rec("FluidFlowSettings");
                self.i(&mut fw, "type", match s.flow_type {
                    FlowType::Smoke => 1,
                    FlowType::Fire => 2,
                    FlowType::Both => 3,
                    FlowType::Liquid => 4,
                });
                self.i(&mut fw, "behavior", match s.flow_behavior {
                    FlowBehavior::Inflow => 0,
                    FlowBehavior::Outflow => 1,
                    FlowBehavior::Geometry => 2,
                });
                for (k, v) in [
                    ("density", s.density),
                    ("fuel_amount", s.fuel_amount),
                    ("temperature", s.temperature),
                    ("volume_density", s.volume_density),
                    ("surface_distance", s.surface_distance),
                    ("vel_multi", s.velocity_factor),
                    ("vel_normal", s.velocity_normal),
                ] {
                    self.f(&mut fw, k, &[v]);
                }
                self.f(&mut fw, "color", &s.smoke_color);
                self.f(&mut fw, "vel_coord", &s.velocity_coord);
                let a = self.data(&[fw]);
                self.p(r, "flow", a);
            }
            FluidType::Effector => {
                self.i(r, "type", 4);
                let mut e = self.rec("FluidEffectorSettings");
                self.i(&mut e, "type", (s.effector_type == EffectorType::Guide) as i64);
                self.f(&mut e, "surface_distance", &[s.effector_surface_distance]);
                let a = self.data(&[e]);
                self.p(r, "effector", a);
            }
        }
    }

    /// A `ParticleSettings` ID (`PA`).
    pub(super) fn particle_settings(&mut self, addr: u64, name: &str, s: &ParticleSettings, ids: &Ids) {
        self.start_id();
        let mut r = self.id("ParticleSettings", "PA", name);
        let hair = s.kind == ParticleType::Hair;
        self.i(&mut r, "type", if hair { 2 } else { 0 });
        self.i(&mut r, "totpart", s.count as i64);
        for (k, v) in [
            ("sta", s.frame_start),
            ("end", s.frame_end),
            ("lifetime", s.lifetime),
            ("randlife", s.lifetime_random),
            ("jitfac", s.jitter_factor),
            ("normfac", if hair { s.hair_length } else { s.normal_factor }),
            ("tanfac", s.tangent_factor),
            ("tanphase", s.tangent_phase),
            ("obfac", s.object_factor),
            ("randfac", s.factor_random),
            ("mass", s.mass),
            ("size", s.particle_size),
            ("randsize", s.size_random),
            ("dragfac", s.drag_factor),
            ("brownfac", s.brownian_factor),
            ("dampfac", s.damping),
            ("draw_size", s.display_size),
            ("rad_root", s.root_radius),
            ("rad_tip", s.tip_radius),
            ("rad_scale", s.radius_scale),
            ("childrad", s.child_radius),
            ("childflat", s.child_roundness),
            ("clumpfac", s.clump_factor),
            ("rough1", s.roughness_1),
        ] {
            self.f(&mut r, k, &[v]);
        }
        self.i(&mut r, "hair_step", s.hair_step as i64);
        self.i(&mut r, "from", match s.emit_from {
            EmitFrom::Vert => 0,
            EmitFrom::Face => 1,
            EmitFrom::Volume => 2,
        });
        self.i(&mut r, "distr", match s.distribution {
            Distribution::Jittered => 0,
            Distribution::Random => 1,
            Distribution::Grid => 2,
        });
        self.i(&mut r, "userjit", s.userjit as i64);
        self.i(&mut r, "grid_res", s.grid_resolution as i64);
        self.f(&mut r, "ob_vel", &s.object_align_factor);
        self.i(&mut r, "phystype", (s.physics_type == ParticlePhysics::Newton) as i64);
        self.i(&mut r, "integrator", match s.integrator {
            Integrator::Euler => 0,
            Integrator::Midpoint => 1,
            Integrator::Rk4 => 2,
            Integrator::Verlet => 3,
        });
        self.i(&mut r, "subframes", s.subframes as i64);
        self.i(&mut r, "ren_as", match s.render_type {
            RenderType::None => 0,
            RenderType::Line => 5,
            RenderType::Path => 6,
            RenderType::Object => 7,
            RenderType::Collection => 8,
            RenderType::Halo => 1,
        });
        self.i(&mut r, "draw_as", match s.display_method {
            DisplayMethod::None => 0,
            DisplayMethod::Point => 1,
            DisplayMethod::Circle => 2,
            DisplayMethod::Cross => 3,
            DisplayMethod::Axis => 4,
            DisplayMethod::Rendered => 5, // PART_DRAW_REND
        });
        self.p(&mut r, "dup_ob", ids.object(&s.instance_object));
        self.p(&mut r, "dup_group", ids.collection(&s.instance_collection));
        self.i(&mut r, "omat", s.material_slot as i64);
        self.i(&mut r, "disp", s.display_percentage as i64);
        self.i(&mut r, "draw_step", s.display_step as i64);
        self.i(&mut r, "ren_step", s.render_step as i64);
        self.i(&mut r, "childtype", match s.child_type {
            ChildType::None => 0,
            ChildType::Simple => 1,
            ChildType::Interpolated => 2,
        });
        self.i(&mut r, "child_nbr", s.child_nbr as i64);
        self.i(&mut r, "ren_child_nbr", s.rendered_child_count as i64);
        self.f(&mut r, "clength", &[s.child_length]);
        self.f(&mut r, "draw_line", &[s.line_length_tail, s.line_length_head]);
        // PART_HIDE_ADVANCED_HAIR (the DNA default) kept.
        let flag = self.get(&r, "flag") & !(1 << 5 | 1 << 6 | 1 << 7 | 1 << 8);
        self.i(&mut r, "flag", flag | bits(&[(s.show_unborn, 1 << 5), (s.use_dead, 1 << 6), (s.use_emit_random, 1 << 7), (s.use_even_distribution, 1 << 8)]));
        self.i(&mut r, "draw", bits(&[(s.use_rotation_instance, 1 << 7), (s.use_collection_pick_random, 1 << 10), (!s.use_scale_instance, 1 << 16)]));
        let ew = self.effector_weights(&s.effector_weights, ids);
        self.p(&mut r, "effector_weights", ew);
        self.put(b"PA", addr, &[r]);
    }
}

/// `RigidBodyCon.type` as the reader maps it.
fn constraint_type(v: i64) -> ConstraintType {
    match v {
        1 => ConstraintType::Fixed,
        2 => ConstraintType::Hinge,
        3 => ConstraintType::Slider,
        4 => ConstraintType::Piston,
        5 => ConstraintType::Generic,
        6 => ConstraintType::GenericSpring,
        7 => ConstraintType::Motor,
        _ => ConstraintType::Point,
    }
}
