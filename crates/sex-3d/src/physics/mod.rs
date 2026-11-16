// SPDX-License-Identifier: GPL-3.0-only
//! Physics: Blender's simulation systems for Polyamour.
//!
//! * [`settings`]: every setting with Blender's names, defaults and panels
//!   ([`props`] turns them into UI panels, polyops options and signatures).
//! * [`cache`]: the point cache (frames in memory, bake, outdated, disk).
//! * [`rigid`]: Rigid Body World, rigid bodies and constraints.
//! * [`cloth`]: Cloth and Soft Body.
//! * [`particles`]: particle systems (emitter and hair).
//! * [`field`]: force fields and gravity.
//! * [`fluid`]: smoke and fire.
//! * [`collision`]: Collision objects as the simulations see them.
//!
//! Settings live on the scene ([`ScenePhysics`]: gravity, the rigid body
//! world), on objects ([`ObjectPhysics`]: rigid body, rigid body constraint,
//! force field) and in the modifier stack ([`PhysicsModifier`]: Cloth, Soft
//! Body, Collision, Particle System and Fluid, as in Blender). [`update`]
//! steps every simulation up to the scene's frame, filling the cache;
//! evaluation (object matrices, modifiers, particles, renders) only reads
//! the cache.
pub mod cache;
pub mod cloth;
pub mod collision;
pub mod disk;
pub mod field;
pub mod fluid;
pub mod particles;
pub mod props;
pub mod rigid;
pub mod settings;

use crate::mesh::Mesh;
use crate::modifiers::{EvalContext, Modifier};
use crate::scene::Scene;
use cache::{Caches, SimFrame};
use props::Props;
pub use settings::*;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Scene-level physics: gravity, the rigid body world and the caches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScenePhysics {
    pub gravity: SceneGravity,
    pub rigidbody_world: Option<RigidBodyWorld>,
    /// Shared runtime caches (not saved; compare equal).
    pub caches: Caches,
}

/// Object-level physics that Blender keeps on the object, not in the stack.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectPhysics {
    pub rigid_body: Option<RigidBody>,
    pub rigid_body_constraint: Option<RigidBodyConstraint>,
    pub field: Option<FieldSettings>,
}

impl ObjectPhysics {
    pub fn is_empty(&self) -> bool {
        self.rigid_body.is_none() && self.rigid_body_constraint.is_none() && self.field.is_none()
    }
}

/// The physics modifiers (Blender's Cloth, Softbody, Collision,
/// ParticleSystem and Fluid modifiers).
#[derive(Clone, Debug, PartialEq)]
pub enum PhysicsModifier {
    Cloth(Box<ClothSettings>),
    SoftBody(Box<SoftBodySettings>),
    Collision(CollisionSettings),
    ParticleSystem(Box<ParticleSystem>),
    Fluid(Box<FluidSettings>),
}

impl PhysicsModifier {
    /// Blender's modifier name.
    pub fn name(&self) -> &'static str {
        match self {
            PhysicsModifier::Cloth(_) => "Cloth",
            PhysicsModifier::SoftBody(_) => "Softbody",
            PhysicsModifier::Collision(_) => "Collision",
            PhysicsModifier::ParticleSystem(_) => "ParticleSystem",
            PhysicsModifier::Fluid(_) => "Fluid",
        }
    }

    /// By Blender type identifier or name.
    pub fn with_defaults(key: &str) -> Option<PhysicsModifier> {
        Some(match key {
            "cloth" => PhysicsModifier::Cloth(Box::default()),
            "softbody" => PhysicsModifier::SoftBody(Box::default()),
            "collision" => PhysicsModifier::Collision(CollisionSettings::default()),
            "particlesystem" => PhysicsModifier::ParticleSystem(Box::default()),
            "fluid" => PhysicsModifier::Fluid(Box::default()),
            _ => return None,
        })
    }

    pub fn props(&mut self) -> &mut dyn Props {
        match self {
            PhysicsModifier::Cloth(s) => &mut **s,
            PhysicsModifier::SoftBody(s) => &mut **s,
            PhysicsModifier::Collision(s) => s,
            PhysicsModifier::ParticleSystem(s) => &mut **s,
            PhysicsModifier::Fluid(s) => &mut **s,
        }
    }

    /// The modifier's place in the stack: the cache the evaluation reads.
    pub(crate) fn apply(&self, md: &Modifier, m: &Mesh, cx: &EvalContext<'_>) -> Mesh {
        let Some(scene) = cx.scene else { return m.clone() };
        let Some(o) = scene.objects.get(cx.owner) else { return m.clone() };
        let slot = o.modifiers.iter().position(|x| std::ptr::eq(x, md)).or_else(|| o.modifiers.iter().position(|x| x == md));
        let Some(slot) = slot else { return m.clone() };
        match self {
            PhysicsModifier::Cloth(s) => cloth::modifier_eval(m, scene, cx.owner, slot, &cloth::cloth_key(&o.name, slot), s.point_cache.frame_start),
            PhysicsModifier::SoftBody(s) => cloth::modifier_eval(m, scene, cx.owner, slot, &cloth::softbody_key(&o.name, slot), s.point_cache.frame_start),
            _ => m.clone(),
        }
    }
}

/// One simulation in the scene, for the cache panels and Bake All.
#[derive(Clone, Debug, PartialEq)]
pub struct SimInfo {
    pub key: String,
    pub object: Option<usize>,
    pub slot: usize,
    pub start: i32,
    pub end: i32,
    pub disk: bool,
}

/// Every simulation in the scene.
pub fn sims(scene: &Scene) -> Vec<SimInfo> {
    let mut out = Vec::new();
    if let Some(w) = &scene.physics.rigidbody_world {
        if w.enabled && scene.objects.iter().any(|o| o.physics.rigid_body.is_some()) {
            out.push(SimInfo { key: rigid::CACHE_KEY.into(), object: None, slot: 0, start: w.point_cache.frame_start, end: w.point_cache.frame_end, disk: w.point_cache.use_disk_cache });
        }
    }
    for (i, o) in scene.objects.iter().enumerate() {
        for (slot, md) in o.modifiers.iter().enumerate() {
            let Modifier::Physics(p) = md else { continue };
            let (key, pc) = match p {
                PhysicsModifier::Cloth(s) => (cloth::cloth_key(&o.name, slot), s.point_cache.clone()),
                PhysicsModifier::SoftBody(s) => (cloth::softbody_key(&o.name, slot), s.point_cache.clone()),
                PhysicsModifier::ParticleSystem(s) if s.settings.kind == ParticleType::Emitter => (particles::key(&o.name, slot), s.point_cache.clone()),
                PhysicsModifier::Fluid(s) if s.fluid_type == FluidType::Domain && s.domain_type == DomainType::Gas => {
                    (fluid::key(&o.name, slot), PointCacheSettings { frame_start: s.cache_frame_start, frame_end: s.cache_frame_end, use_disk_cache: false })
                }
                _ => continue,
            };
            out.push(SimInfo { key, object: Some(i), slot, start: pc.frame_start, end: pc.frame_end, disk: pc.use_disk_cache });
        }
    }
    out
}

/// True when anything in the scene simulates (the app steps it on frame
/// changes).
pub fn has_sims(scene: &Scene) -> bool {
    !sims(scene).is_empty()
}

fn hash_mesh(h: &mut impl Hasher, m: &Mesh) {
    m.positions.len().hash(h);
    for p in &m.positions {
        p.to_array().map(f32::to_bits).hash(h);
    }
    m.faces.len().hash(h);
    for f in &m.faces {
        f.verts.hash(h);
    }
    m.vertex_groups.hash(h);
    for w in &m.weights {
        for (g, x) in w {
            (g, x.to_bits()).hash(h);
        }
    }
}

/// Settings of everything a modifier simulation reacts to: collision
/// objects, fields, gravity.
fn hash_environment(h: &mut impl Hasher, scene: &Scene) {
    scene.physics.gravity.use_gravity.hash(h);
    scene.physics.gravity.gravity.map(f32::to_bits).hash(h);
    scene.fps.to_bits().hash(h);
    for o in &scene.objects {
        if let Some(f) = &o.physics.field {
            o.name.hash(h);
            props::hash_into(&mut f.clone(), h);
        }
        for md in &o.modifiers {
            if let Modifier::Physics(p @ (PhysicsModifier::Collision(_) | PhysicsModifier::Fluid(_))) = md {
                o.name.hash(h);
                props::hash_into(p.clone().props(), h);
            }
        }
    }
}

fn hash_start(h: &mut impl Hasher, scene: &Scene) {
    for i in 0..scene.objects.len() {
        let o = &scene.objects[i];
        if o.physics.field.is_some() || o.modifiers.iter().any(|m| matches!(m, Modifier::Physics(_))) {
            scene.world_matrix(i).to_cols_array().map(f32::to_bits).hash(h);
        }
    }
}

/// How far [`update`] got.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpdateReport {
    /// Frames simulated.
    pub stepped: usize,
    /// Caches freed because settings changed.
    pub invalidated: usize,
}

/// Simulate every simulation up to the scene's current frame (stepping
/// sequentially from the last cached frame, at most `max_steps` frames per
/// simulation). Jumping before a simulation's start shows the unsimulated
/// scene; outdated caches are freed there.
pub fn update(scene: &Scene) -> UpdateReport {
    update_limited(scene, usize::MAX)
}

pub fn update_limited(scene: &Scene, max_steps: usize) -> UpdateReport {
    let mut rep = UpdateReport::default();
    let caches = &scene.physics.caches;
    let frame = scene.frame_current;
    // Rigid bodies first: cloth and particles collide with where they are.
    if let Some(w) = &scene.physics.rigidbody_world {
        if w.enabled && scene.objects.iter().any(|o| o.physics.rigid_body.is_some()) {
            let (sig, init) = rigid::signatures(scene);
            let pc = &w.point_cache;
            rep.add(drive(
                caches,
                rigid::CACHE_KEY,
                sig,
                init,
                pc.frame_start,
                pc.frame_end,
                frame,
                max_steps,
                false,
                &|| Some(SimFrame::Rigid(rigid::initial_frame(scene))),
                &|prev, _f| match prev {
                    SimFrame::Rigid(r) => Some(SimFrame::Rigid(rigid::step_frame(scene, r))),
                    _ => None,
                },
            ));
        }
    }
    for i in 0..scene.objects.len() {
        let o = &scene.objects[i];
        for (slot, md) in o.modifiers.iter().enumerate() {
            let Modifier::Physics(p) = md else { continue };
            let mut h = std::collections::hash_map::DefaultHasher::new();
            let mut hi = std::collections::hash_map::DefaultHasher::new();
            slot.hash(&mut h);
            props::hash_into(p.clone().props(), &mut h);
            if let crate::scene::ObjectData::Mesh(mi) = o.data {
                hash_mesh(&mut h, &scene.meshes[mi]);
            }
            hash_environment(&mut h, scene);
            hash_start(&mut hi, scene);
            let (sig, init) = (h.finish(), hi.finish());
            match p {
                PhysicsModifier::Cloth(s) => {
                    let key = cloth::cloth_key(&o.name, slot);
                    rep.add(drive(
                        caches,
                        &key,
                        sig,
                        init,
                        s.point_cache.frame_start,
                        s.point_cache.frame_end,
                        frame,
                        max_steps,
                        false,
                        &|| cloth::initial(scene, i, slot).map(SimFrame::Points),
                        &|prev, _| match prev {
                            SimFrame::Points(pf) => cloth::cloth_step(scene, i, slot, s, pf).map(SimFrame::Points),
                            _ => None,
                        },
                    ));
                }
                PhysicsModifier::SoftBody(s) => {
                    let key = cloth::softbody_key(&o.name, slot);
                    rep.add(drive(
                        caches,
                        &key,
                        sig,
                        init,
                        s.point_cache.frame_start,
                        s.point_cache.frame_end,
                        frame,
                        max_steps,
                        false,
                        &|| cloth::initial(scene, i, slot).map(SimFrame::Points),
                        &|prev, _| match prev {
                            SimFrame::Points(pf) => cloth::softbody_step(scene, i, slot, s, pf).map(SimFrame::Points),
                            _ => None,
                        },
                    ));
                }
                PhysicsModifier::ParticleSystem(sys) if sys.settings.kind == ParticleType::Emitter => {
                    let key = particles::key(&o.name, slot);
                    let start = sys.point_cache.frame_start;
                    rep.add(drive(
                        caches,
                        &key,
                        sig,
                        init,
                        start,
                        sys.point_cache.frame_end,
                        frame,
                        max_steps,
                        false,
                        &|| {
                            let p0 = particles::initial(scene, i, slot, sys);
                            Some(SimFrame::Particles(particles::step(scene, i, slot, sys, &p0, start)))
                        },
                        &|prev, f| match prev {
                            SimFrame::Particles(pf) => Some(SimFrame::Particles(particles::step(scene, i, slot, sys, pf, f))),
                            _ => None,
                        },
                    ));
                }
                PhysicsModifier::Fluid(s) if s.fluid_type == FluidType::Domain && s.domain_type == DomainType::Gas => {
                    let key = fluid::key(&o.name, slot);
                    rep.add(drive(
                        caches,
                        &key,
                        sig,
                        init,
                        s.cache_frame_start,
                        s.cache_frame_end,
                        frame,
                        max_steps,
                        true,
                        &|| fluid::initial(scene, i, s).map(SimFrame::Fluid),
                        &|prev, _| match prev {
                            SimFrame::Fluid(ff) => fluid::step(scene, i, s, ff).map(SimFrame::Fluid),
                            _ => None,
                        },
                    ));
                }
                _ => {}
            }
        }
    }
    rep
}

impl UpdateReport {
    fn add(&mut self, o: UpdateReport) {
        self.stepped += o.stepped;
        self.invalidated += o.invalidated;
    }
}

fn frame_bytes(f: &SimFrame) -> usize {
    match f {
        SimFrame::Rigid(r) => r.bytes(),
        SimFrame::Points(p) => p.bytes(),
        SimFrame::Particles(p) => p.bytes(),
        SimFrame::Fluid(f) => f.bytes(),
    }
}

/// The point cache's stepping rules for one simulation. `resume_only`:
/// cached frames drop the full state (fluids) and stepping continues from
/// the separately kept last state.
#[allow(clippy::too_many_arguments)]
fn drive(
    c: &Caches,
    key: &str,
    sig: u64,
    init_sig: u64,
    start: i32,
    end: i32,
    frame: i32,
    max_steps: usize,
    resume_only: bool,
    init: &dyn Fn() -> Option<SimFrame>,
    step: &dyn Fn(&SimFrame, i32) -> Option<SimFrame>,
) -> UpdateReport {
    let mut rep = UpdateReport::default();
    if c.validate(key, sig) {
        rep.invalidated += 1;
    }
    if frame == start && c.validate_start(key, init_sig) {
        rep.invalidated += 1;
    }
    if c.is_baked(key) {
        return rep;
    }
    if frame <= start {
        if c.entry(key).is_some_and(|e| e.outdated) {
            c.free(key);
        }
    }
    if frame < start || end < start {
        return rep;
    }
    let target = frame.min(end);
    if c.exact(key, target).is_some() {
        return rep;
    }
    let prepare = |f: SimFrame| -> SimFrame {
        match (&f, resume_only) {
            (SimFrame::Fluid(ff), true) => SimFrame::Fluid(fluid::display_copy(ff)),
            _ => f,
        }
    };
    // Where to continue from.
    let (mut f, mut state) = match c.last(key) {
        Some((lf, data)) if lf < target => {
            if resume_only {
                match c.resume(key).filter(|r| r.0 == lf) {
                    Some((_, full)) => (lf, full),
                    None => {
                        // Lost the full state: start over.
                        c.truncate_after(key, start - 1);
                        let Some(first) = init() else { return rep };
                        let first = Arc::new(first);
                        c.insert(key, start, prepare((*first).clone()), frame_bytes(&first));
                        c.set_resume(key, start, first.clone());
                        (start, first)
                    }
                }
            } else {
                (lf, data)
            }
        }
        Some(_) => return rep,
        None => {
            let Some(first) = init() else { return rep };
            let first = Arc::new(first);
            c.insert(key, start, prepare((*first).clone()), frame_bytes(&first));
            if resume_only {
                c.set_resume(key, start, first.clone());
            }
            (start, first)
        }
    };
    let mut n = 0;
    while f < target && n < max_steps {
        let Some(next) = step(&state, f + 1) else { break };
        f += 1;
        n += 1;
        let next = Arc::new(next);
        c.insert(key, f, prepare((*next).clone()), frame_bytes(&next));
        if resume_only {
            c.set_resume(key, f, next.clone());
        }
        state = next;
    }
    rep.stepped += n;
    // Frames left after a free were simulated since: once they reach the
    // current frame again, the cache is no longer outdated.
    if f >= target {
        c.mark_current(key);
    }
    rep
}

/// Bake the simulations named by `keys` (all when empty) over their frame
/// ranges. `pose` puts the scene at a frame (the app evaluates animation
/// there); the scene's frame is restored afterwards. The caller writes
/// [`disk_files`] for Disk Cache simulations.
pub fn bake(scene: &mut Scene, keys: &[String], pose: &mut dyn FnMut(&mut Scene, i32)) -> Result<usize, String> {
    let all = sims(scene);
    let chosen: Vec<SimInfo> = all.into_iter().filter(|s| keys.is_empty() || keys.contains(&s.key)).collect();
    if chosen.is_empty() {
        return Ok(0);
    }
    let caches = scene.physics.caches.clone();
    for s in &chosen {
        caches.free(&s.key);
    }
    let start = chosen.iter().map(|s| s.start).min().unwrap_or(1);
    let end = chosen.iter().map(|s| s.end).max().unwrap_or(start);
    let saved = scene.frame_current;
    for f in start..=end {
        scene.frame_current = f;
        pose(scene, f);
        update(scene);
    }
    scene.frame_current = saved;
    pose(scene, saved);
    for s in &chosen {
        caches.set_baked(&s.key, true);
        caches.mark_current(&s.key);
    }
    Ok(chosen.len())
}

/// Free the bakes named by `keys` (all when empty): Blender's Free Bake and
/// Delete All Bakes. Returns the disk cache file names to delete.
pub fn free_bakes(scene: &Scene, keys: &[String]) -> Vec<String> {
    let caches = &scene.physics.caches;
    let mut files = Vec::new();
    for s in sims(scene) {
        if keys.is_empty() || keys.contains(&s.key) {
            caches.free(&s.key);
            if s.disk {
                files.push(disk::file_name(&s.key));
            }
        }
    }
    files
}

/// The disk cache files of baked Disk Cache simulations: (file name in
/// the `cache` folder, bytes).
pub fn disk_files(scene: &Scene) -> Vec<(String, Vec<u8>)> {
    let caches = &scene.physics.caches;
    sims(scene).into_iter().filter(|s| s.disk && caches.is_baked(&s.key)).filter_map(|s| Some((disk::file_name(&s.key), disk::encode(caches, &s.key)?))).collect()
}

/// Read disk-baked caches back (after opening a file); `read` gets a file
/// name in the `cache` folder.
pub fn load_disk_caches(scene: &Scene, read: &dyn Fn(&str) -> Option<Vec<u8>>) -> usize {
    let mut n = 0;
    for s in sims(scene) {
        if s.disk {
            if let Some(b) = read(&disk::file_name(&s.key)) {
                if disk::decode(&scene.physics.caches, &s.key, &b) {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Cache key of the simulation of object `obj`'s modifier at `slot`.
pub fn modifier_key(scene: &Scene, obj: usize, slot: usize) -> Option<String> {
    sims(scene).into_iter().find(|s| s.object == Some(obj) && s.slot == slot).map(|s| s.key)
}
