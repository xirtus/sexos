// SPDX-License-Identifier: GPL-3.0-only
//! Physics in the editor: the operators behind the Physics, Particles and
//! Scene tabs (enabling and removing simulations, editing their settings,
//! Bake / Free Bake / Bake All / Delete All Bakes), Object > Rigid Body and
//! Object > Quick Effects, stepping the simulations as the frame changes,
//! and the Disk Cache files in `cache/` next to the `.poly` file.
//!
//! The settings and simulations are sex-3d's ([`sex_3d::physics`]); the
//! editor only steps them: a few frames per tick ([`STEPS`]) so that a jump
//! far ahead in the timeline keeps the window responsive while the cache
//! fills.
use crate::doc::Doc;
use crate::ui::{MenuItem, Op};
use sex_3d::math::Vec3;
use sex_3d::modifiers::Modifier;
use sex_3d::physics::props::{self, PropEnum, Props};
use sex_3d::physics::{
    self as ph, CollisionShape, ConstraintType, DomainType, FieldSettings, FieldType, FluidSettings, FluidType, ParticleSettings, ParticleSystem, PhysicsModifier, RigidBody, RigidBodyConstraint, RigidBodyType,
    RigidBodyWorld,
};
use sex_3d::scene::{Object, ObjectData, Scene};
use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// Frames each simulation may advance per editor tick.
pub const STEPS: usize = 8;

/// The editor's simulation state (not saved, not undone).
#[derive(Clone, Debug, Default)]
pub struct SimState {
    /// Frame and document generation of the last update.
    pub last: Option<(i32, u64)>,
    /// The last update stopped short of the current frame.
    pub pending: bool,
    /// Active particle system per object (Particles tab list).
    pub active_particles: BTreeMap<String, usize>,
}

/// What a settings panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Gravity,
    World,
    RigidBody(usize),
    Constraint(usize),
    Field(usize),
    /// Object and modifier slot.
    Modifier(usize, usize),
}

impl Target {
    pub fn encode(self) -> String {
        match self {
            Target::Gravity => "gravity".into(),
            Target::World => "world".into(),
            Target::RigidBody(o) => format!("rb:{o}"),
            Target::Constraint(o) => format!("rbc:{o}"),
            Target::Field(o) => format!("field:{o}"),
            Target::Modifier(o, s) => format!("mod:{o}:{s}"),
        }
    }
    pub fn parse(s: &str) -> Option<Target> {
        let mut it = s.split(':');
        let kind = it.next()?;
        let mut num = || it.next().and_then(|x| x.parse::<usize>().ok());
        Some(match kind {
            "gravity" => Target::Gravity,
            "world" => Target::World,
            "rb" => Target::RigidBody(num()?),
            "rbc" => Target::Constraint(num()?),
            "field" => Target::Field(num()?),
            "mod" => {
                let o = num()?;
                Target::Modifier(o, num()?)
            }
            _ => return None,
        })
    }
    /// The simulation whose cache this target's Cache panel shows.
    pub fn cache_key(self, scene: &Scene) -> Option<String> {
        match self {
            Target::World => Some(ph::rigid::CACHE_KEY.to_string()),
            Target::Modifier(o, s) => ph::modifier_key(scene, o, s),
            _ => None,
        }
    }
}

/// Run `f` on the settings `t` names, if they exist.
pub fn with_props<R>(scene: &mut Scene, t: Target, f: impl FnOnce(&mut dyn Props) -> R) -> Option<R> {
    match t {
        Target::Gravity => Some(f(&mut scene.physics.gravity)),
        Target::World => scene.physics.rigidbody_world.as_mut().map(|w| f(w)),
        Target::RigidBody(o) => scene.objects.get_mut(o)?.physics.rigid_body.as_mut().map(|x| f(x)),
        Target::Constraint(o) => scene.objects.get_mut(o)?.physics.rigid_body_constraint.as_mut().map(|x| f(x)),
        Target::Field(o) => scene.objects.get_mut(o)?.physics.field.as_mut().map(|x| f(x)),
        Target::Modifier(o, s) => match scene.objects.get_mut(o)?.modifiers.get_mut(s)? {
            Modifier::Physics(p) => Some(f(p.props())),
            _ => None,
        },
    }
}

/// The Physics tab's buttons: (operator argument, label).
pub const KINDS: [(&str, &str); 8] = [
    ("FORCE_FIELD", "Force Field"),
    ("COLLISION", "Collision"),
    ("CLOTH", "Cloth"),
    ("DYNAMIC_PAINT", "Dynamic Paint"),
    ("SOFT_BODY", "Soft Body"),
    ("FLUID", "Fluid"),
    ("RIGID_BODY", "Rigid Body"),
    ("RIGID_BODY_CONSTRAINT", "Rigid Body Constraint"),
];

fn modifier_slot(o: &Object, kind: &str) -> Option<usize> {
    o.modifiers.iter().position(|m| match m {
        Modifier::Physics(p) => matches!((kind, p), ("COLLISION", PhysicsModifier::Collision(_)) | ("CLOTH", PhysicsModifier::Cloth(_)) | ("SOFT_BODY", PhysicsModifier::SoftBody(_)) | ("FLUID", PhysicsModifier::Fluid(_)) | ("DYNAMIC_PAINT", PhysicsModifier::DynamicPaint(_))),
        _ => false,
    })
}

/// Whether physics `kind` (a [`KINDS`] argument) is on for object `o`.
pub fn has_kind(o: &Object, kind: &str) -> bool {
    match kind {
        "FORCE_FIELD" => o.physics.field.is_some(),
        "RIGID_BODY" => o.physics.rigid_body.is_some(),
        "RIGID_BODY_CONSTRAINT" => o.physics.rigid_body_constraint.is_some(),
        _ => modifier_slot(o, kind).is_some(),
    }
}

/// Which physics an object type can take (Blender: meshes take all, empties
/// fields and rigid body constraints).
pub fn kind_allowed(o: &Object, kind: &str) -> bool {
    match o.data {
        ObjectData::Mesh(_) => true,
        _ => matches!(kind, "FORCE_FIELD" | "RIGID_BODY_CONSTRAINT"),
    }
}

fn ensure_world(scene: &mut Scene) {
    if scene.physics.rigidbody_world.is_none() {
        let mut w = RigidBodyWorld::default();
        w.point_cache.frame_end = scene.frame_end;
        scene.physics.rigidbody_world = Some(w);
    }
}

fn add_rigid_body(scene: &mut Scene, i: usize, kind: RigidBodyType) {
    ensure_world(scene);
    let o = &mut scene.objects[i];
    if !matches!(o.data, ObjectData::Mesh(_)) {
        return;
    }
    let rb = o.physics.rigid_body.get_or_insert_with(RigidBody::default);
    rb.kind = kind;
}

fn mesh_selection(doc: &Doc) -> Vec<usize> {
    doc.scene.selected.iter().copied().filter(|&i| i < doc.scene.objects.len() && matches!(doc.scene.objects[i].data, ObjectData::Mesh(_))).collect()
}

fn toggle(doc: &mut Doc, kind: &str) -> bool {
    let Some(i) = doc.active() else {
        doc.report = "No active object".into();
        return true;
    };
    let label = KINDS.iter().find(|k| k.0 == kind).map_or(kind, |k| k.1);
    if !kind_allowed(&doc.scene.objects[i], kind) {
        doc.report = format!("{label} needs a mesh object");
        return true;
    }
    let on = has_kind(&doc.scene.objects[i], kind);
    doc.push_undo(&format!("{} {label}", if on { "Remove" } else { "Add" }));
    let scene = &mut doc.scene;
    match (kind, on) {
        ("FORCE_FIELD", true) => scene.objects[i].physics.field = None,
        ("FORCE_FIELD", false) => scene.objects[i].physics.field = Some(FieldSettings::new(FieldType::Force)),
        ("RIGID_BODY", true) => scene.objects[i].physics.rigid_body = None,
        ("RIGID_BODY", false) => add_rigid_body(scene, i, RigidBodyType::Active),
        ("RIGID_BODY_CONSTRAINT", true) => scene.objects[i].physics.rigid_body_constraint = None,
        ("RIGID_BODY_CONSTRAINT", false) => {
            ensure_world(scene);
            scene.objects[i].physics.rigid_body_constraint = Some(RigidBodyConstraint { kind: ConstraintType::Fixed, ..Default::default() });
        }
        (_, true) => {
            if let Some(s) = modifier_slot(&scene.objects[i], kind) {
                scene.objects[i].modifiers.remove(s);
            }
        }
        (_, false) => {
            let m = match kind {
                "COLLISION" => PhysicsModifier::with_defaults("collision"),
                "CLOTH" => PhysicsModifier::with_defaults("cloth"),
                "SOFT_BODY" => PhysicsModifier::with_defaults("softbody"),
                "FLUID" => PhysicsModifier::with_defaults("fluid"),
                "DYNAMIC_PAINT" => PhysicsModifier::with_defaults("dynamicpaint"),
                _ => None,
            };
            if let Some(m) = m {
                scene.objects[i].modifiers.push(Modifier::Physics(m));
            }
        }
    }
    doc.touch();
    true
}

/// Blender's Calculate Mass presets: (name, density in kg/m³).
pub const DENSITIES: [(&str, f32); 12] = [
    ("Air", 1.0),
    ("Acrylic", 1400.0),
    ("Asphalt (Crushed)", 721.0),
    ("Bark", 240.0),
    ("Brick (Common)", 2000.0),
    ("Concrete", 2320.0),
    ("Glass (Broken)", 1940.0),
    ("Gold", 19320.0),
    ("Ice", 916.7),
    ("Rubber", 1522.0),
    ("Steel", 7860.0),
    ("Wood", 600.0),
];

/// Object > Rigid Body.
pub fn rigid_body_menu() -> Vec<MenuItem> {
    let op = |l: &str, id: &str, a: &str| MenuItem::op(l, "", Op::with(id, a));
    let shapes = CollisionShape::ITEMS.iter().map(|s| op(s.2, "rigidbody.shape_change", s.1)).collect();
    let masses = DENSITIES.iter().map(|(n, d)| op(n, "rigidbody.mass_calculate", &d.to_string())).collect();
    let connect = ConstraintType::ITEMS.iter().map(|s| op(s.2, "rigidbody.connect", s.1)).collect();
    vec![
        op("Add Active", "rigidbody.objects_add", "ACTIVE"),
        op("Add Passive", "rigidbody.objects_add", "PASSIVE"),
        op("Remove", "rigidbody.objects_remove", ""),
        MenuItem::Separator,
        MenuItem::Sub("Change Shape".into(), shapes),
        MenuItem::Sub("Calculate Mass".into(), masses),
        MenuItem::Separator,
        op("Bake to Keyframes", "rigidbody.bake_to_keyframes", ""),
        MenuItem::Sub("Connect".into(), connect),
    ]
}

/// Object > Quick Effects.
pub fn quick_effects_menu() -> Vec<MenuItem> {
    vec![MenuItem::op("Quick Fur", "", Op::new("object.quick_fur")), MenuItem::op("Quick Smoke", "", Op::new("object.quick_smoke")), MenuItem::op("Quick Liquid", "", Op::new("object.quick_liquid"))]
}

/// The physics operators. Returns false for anything else.
pub fn run(doc: &mut Doc, op: &Op) -> bool {
    let arg = op.arg.as_str();
    match op.id.as_str() {
        "physics.set" => set(doc, arg),
        "physics.toggle" => toggle(doc, arg),
        "rigidbody.world_add" => {
            doc.push_undo("Add Rigid Body World");
            ensure_world(&mut doc.scene);
            doc.touch();
            true
        }
        "rigidbody.world_remove" => {
            doc.push_undo("Remove Rigid Body World");
            doc.scene.physics.rigidbody_world = None;
            doc.touch();
            true
        }
        "rigidbody.objects_add" | "rigidbody.object_add" => {
            let kind = RigidBodyType::parse(arg).unwrap_or_default();
            let sel = if op.id == "rigidbody.object_add" { doc.active().into_iter().collect() } else { mesh_selection(doc) };
            if sel.is_empty() {
                doc.report = "No mesh objects selected".into();
                return true;
            }
            doc.push_undo("Add Rigid Bodies");
            for i in sel {
                add_rigid_body(&mut doc.scene, i, kind);
            }
            doc.touch();
            true
        }
        "rigidbody.objects_remove" | "rigidbody.object_remove" => {
            doc.push_undo("Remove Rigid Bodies");
            for i in doc.scene.selected.clone() {
                if let Some(o) = doc.scene.objects.get_mut(i) {
                    o.physics.rigid_body = None;
                }
            }
            doc.touch();
            true
        }
        "rigidbody.shape_change" => {
            let Some(shape) = CollisionShape::parse(arg) else { return true };
            doc.push_undo("Change Collision Shape");
            for i in doc.scene.selected.clone() {
                if let Some(rb) = doc.scene.objects.get_mut(i).and_then(|o| o.physics.rigid_body.as_mut()) {
                    rb.collision_shape = shape;
                }
            }
            doc.touch();
            true
        }
        "rigidbody.mass_calculate" => {
            let density: f32 = arg.parse().unwrap_or(1.0);
            doc.push_undo("Calculate Mass");
            let mut n = 0;
            for i in doc.scene.selected.clone() {
                if doc.scene.objects.get(i).is_some_and(|o| o.physics.rigid_body.is_some()) {
                    if let Some(m) = ph::rigid::calculate_mass(&doc.scene, i, density) {
                        doc.scene.objects[i].physics.rigid_body.as_mut().unwrap().mass = m.max(0.001);
                        n += 1;
                    }
                }
            }
            doc.report = format!("Calculated the mass of {n} object{}", if n == 1 { "" } else { "s" });
            doc.touch();
            true
        }
        "rigidbody.bake_to_keyframes" => bake_to_keyframes(doc),
        "rigidbody.connect" => connect(doc, arg),
        "ptcache.bake" => bake(doc, if arg.is_empty() { vec![] } else { vec![arg.to_string()] }, false),
        "ptcache.bake_all" => bake(doc, vec![], true),
        "ptcache.free_bake" => free(doc, if arg.is_empty() { vec![] } else { vec![arg.to_string()] }, false),
        "ptcache.free_bake_all" => free(doc, vec![], true),
        "object.particle_system_add" => particle_add(doc),
        "object.particle_system_remove" => particle_remove(doc),
        "physics.particle_active" => {
            if let (Some(i), Ok(k)) = (doc.active(), arg.parse()) {
                let name = doc.scene.objects[i].name.clone();
                doc.sim.active_particles.insert(name, k);
            }
            true
        }
        "object.quick_smoke" => quick_smoke(doc),
        "object.quick_liquid" => quick_liquid(doc),
        "fluid.bake_all" | "fluid.bake_data" | "fluid.bake_mesh" => fluid_bake(doc, arg),
        "fluid.free_all" | "fluid.free_data" | "fluid.free_mesh" => fluid_free(doc, arg),
        "fluid.pause_bake" => {
            doc.report = "Nothing is baking".into();
            true
        }
        "dpaint.type_toggle" => dpaint_toggle(doc, arg),
        "object.quick_fur" => quick_fur(doc),
        _ => false,
    }
}

/// `physics.set TARGET\tID\tVALUE`: one property from polyops text.
fn set(doc: &mut Doc, arg: &str) -> bool {
    let mut it = arg.splitn(3, '\t');
    let (Some(t), Some(id), Some(v)) = (it.next().and_then(Target::parse), it.next(), it.next()) else { return true };
    doc.push_undo(&format!("Change {id}"));
    match with_props(&mut doc.scene, t, |p| props::set_text(p, id, v)) {
        Some(Err(e)) => doc.report = e,
        Some(Ok(false)) | None => doc.report = format!("No property {id}"),
        Some(Ok(true)) => {}
    }
    doc.touch();
    true
}

/// The `cache` folder next to the document, when it has been saved.
pub fn cache_dir(doc: &Doc) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(doc.path.as_deref()?);
    Some(p.parent().map(|d| d.to_path_buf()).unwrap_or_default().join("cache"))
}

/// Write the Disk Cache bakes (on save and after a bake). Returns the
/// number of files written.
pub fn write_disk_caches(doc: &Doc) -> Result<usize, String> {
    let files = ph::disk_files(&doc.scene);
    if files.is_empty() {
        return Ok(0);
    }
    let Some(dir) = cache_dir(doc) else { return Ok(0) };
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    for (name, bytes) in &files {
        let p = dir.join(name);
        std::fs::write(&p, bytes).map_err(|e| format!("Cannot write {}: {e}", p.display()))?;
    }
    Ok(files.len())
}

/// Read the Disk Cache bakes back after opening a file.
pub fn load_disk_caches(doc: &Doc) -> usize {
    let Some(dir) = cache_dir(doc) else { return 0 };
    ph::load_disk_caches(&doc.scene, &|name| std::fs::read(dir.join(name)).ok())
}

fn bake(doc: &mut Doc, keys: Vec<String>, all: bool) -> bool {
    let Doc { scene, anim, .. } = doc;
    let anim = &*anim;
    let r = ph::bake(scene, &keys, &mut |s, f| *s = crate::anim::scene_at(anim, s, f as f32, false));
    // The scene was posed at the current frame by the animation again.
    crate::anim::evaluate(doc);
    doc.report = match r {
        Ok(0) => "Nothing to bake".into(),
        Ok(n) => {
            let disk = match write_disk_caches(doc) {
                Ok(0) => String::new(),
                Ok(k) => format!(", {k} written to the cache folder"),
                Err(e) => format!(" ({e})"),
            };
            format!("Baked {n} simulation{}{}{disk}", if n == 1 { "" } else { "s" }, if all { " (Bake All)" } else { "" })
        }
        Err(e) => e,
    };
    doc.sim.last = None;
    doc.touch();
    true
}

fn free(doc: &mut Doc, keys: Vec<String>, all: bool) -> bool {
    let files = ph::free_bakes(&doc.scene, &keys);
    if let Some(dir) = cache_dir(doc) {
        for f in files {
            let _ = std::fs::remove_file(dir.join(f));
        }
    }
    doc.report = if all { "Deleted all bakes".into() } else { "Freed the bake".into() };
    doc.sim.last = None;
    doc.touch();
    true
}

/// Step the simulations towards the current frame (from the per-frame
/// tick). Returns true when the scene changed.
pub fn step(doc: &mut Doc) -> bool {
    if !ph::has_sims(&doc.scene) {
        doc.sim.pending = false;
        doc.sim.last = None;
        return false;
    }
    let frame = doc.scene.frame_current;
    if doc.sim.last == Some((frame, doc.generation)) && !doc.sim.pending {
        return false;
    }
    let frame_changed = doc.sim.last.map(|l| l.0) != Some(frame);
    let r = ph::update_limited(&doc.scene, STEPS);
    doc.sim.pending = r.stepped >= STEPS;
    let changed = frame_changed || r.stepped > 0 || r.invalidated > 0;
    if changed {
        doc.touch();
    }
    doc.sim.last = Some((frame, doc.generation));
    changed
}

/// Whether a simulation still has frames to step (the window keeps
/// ticking).
pub fn pending(doc: &Doc) -> bool {
    doc.sim.pending
}

fn bake_to_keyframes(doc: &mut Doc) -> bool {
    let Some(w) = &doc.scene.physics.rigidbody_world else {
        doc.report = "No Rigid Body World".into();
        return true;
    };
    let (start, end) = (w.point_cache.frame_start, w.point_cache.frame_end);
    let objs: Vec<usize> = doc.scene.selected.iter().copied().filter(|&i| doc.scene.objects.get(i).and_then(|o| o.physics.rigid_body.as_ref()).is_some_and(ph::rigid::is_simulated)).collect();
    if objs.is_empty() {
        doc.report = "No simulated rigid bodies selected".into();
        return true;
    }
    doc.push_undo("Bake to Keyframes");
    let mut keys: Vec<(usize, i32, [f32; 3], [f32; 3])> = Vec::new();
    for f in start..=end {
        let s = crate::anim::scene_at(&doc.anim, &doc.scene, f as f32, false);
        ph::update(&s);
        for &i in &objs {
            let (t, r, _) = s.world_matrix(i).to_trs();
            keys.push((i, f, t.to_array(), sex_3d::script::quat_to_euler_xyz(r)));
        }
    }
    for (i, f, t, r) in keys {
        for k in 0..3 {
            crate::anim::insert_key_value(doc, i, "location", k, f as f32, t[k]);
            crate::anim::insert_key_value(doc, i, "rotation_euler", k, f as f32, r[k]);
        }
    }
    for &i in &objs {
        doc.scene.objects[i].physics.rigid_body = None;
    }
    doc.anim.dirty = true;
    crate::anim::evaluate(doc);
    doc.report = format!("Baked {} object{} to keyframes ({start}-{end})", objs.len(), if objs.len() == 1 { "" } else { "s" });
    doc.touch();
    true
}

/// Connect: an Empty with a rigid body constraint between the active
/// object and each other selected one, at their midpoint.
fn connect(doc: &mut Doc, arg: &str) -> bool {
    let kind = ConstraintType::parse(arg).unwrap_or(ConstraintType::Fixed);
    let Some(a) = doc.active() else {
        doc.report = "No active object".into();
        return true;
    };
    let others: Vec<usize> = doc.scene.selected.iter().copied().filter(|&i| i != a && i < doc.scene.objects.len()).collect();
    if others.is_empty() {
        doc.report = "Select at least two objects".into();
        return true;
    }
    doc.push_undo("Connect Rigid Bodies");
    ensure_world(&mut doc.scene);
    let pa = doc.scene.world_matrix(a).transform_point(Vec3::ZERO);
    let an = doc.scene.objects[a].name.clone();
    let mut made = Vec::new();
    for b in others {
        let pb = doc.scene.world_matrix(b).transform_point(Vec3::ZERO);
        let mut o = Object::new("Constraint", ObjectData::Empty);
        o.transform.location = (pa + pb) * 0.5;
        o.physics.rigid_body_constraint = Some(RigidBodyConstraint { kind, object1: an.clone(), object2: doc.scene.objects[b].name.clone(), ..Default::default() });
        made.push(doc.scene.add_object(o));
    }
    doc.report = format!("Connected with {} constraint{}", made.len(), if made.len() == 1 { "" } else { "s" });
    doc.touch();
    true
}

fn unique_system_name(o: &Object) -> String {
    let taken = |n: &str| o.modifiers.iter().any(|m| matches!(m, Modifier::Physics(PhysicsModifier::ParticleSystem(p)) if p.name == n));
    sex_3d::scene::unique_name("ParticleSystem", taken)
}

fn add_system(scene: &mut Scene, i: usize, settings: ParticleSettings) -> usize {
    let o = &mut scene.objects[i];
    let name = unique_system_name(o);
    let settings = ParticleSettings { name: name.replace("ParticleSystem", "ParticleSettings"), ..settings };
    o.modifiers.push(Modifier::Physics(PhysicsModifier::ParticleSystem(Box::new(ParticleSystem { name, settings, ..Default::default() }))));
    o.modifiers.len() - 1
}

/// The particle systems of object `i`: modifier slots in stack order.
pub fn particle_slots(o: &Object) -> Vec<usize> {
    o.modifiers.iter().enumerate().filter(|(_, m)| matches!(m, Modifier::Physics(PhysicsModifier::ParticleSystem(_)))).map(|(k, _)| k).collect()
}

/// The active particle system's index into [`particle_slots`].
pub fn active_particle(doc: &Doc, o: usize) -> usize {
    let ob = &doc.scene.objects[o];
    let n = particle_slots(ob).len();
    doc.sim.active_particles.get(&ob.name).copied().unwrap_or(0).min(n.saturating_sub(1))
}

fn particle_add(doc: &mut Doc) -> bool {
    let Some(i) = doc.active().filter(|&i| matches!(doc.scene.objects[i].data, ObjectData::Mesh(_))) else {
        doc.report = "Particle systems need a mesh object".into();
        return true;
    };
    doc.push_undo("Add Particle System");
    let end = doc.scene.frame_end;
    let mut s = ParticleSettings::default();
    s.frame_end = (end as f32).min(s.frame_end);
    add_system(&mut doc.scene, i, s);
    let n = particle_slots(&doc.scene.objects[i]).len();
    let name = doc.scene.objects[i].name.clone();
    doc.sim.active_particles.insert(name, n - 1);
    doc.touch();
    true
}

fn particle_remove(doc: &mut Doc) -> bool {
    let Some(i) = doc.active() else { return true };
    let slots = particle_slots(&doc.scene.objects[i]);
    if slots.is_empty() {
        return true;
    }
    let k = active_particle(doc, i);
    doc.push_undo("Remove Particle System");
    doc.scene.objects[i].modifiers.remove(slots[k]);
    let name = doc.scene.objects[i].name.clone();
    doc.sim.active_particles.insert(name, k.saturating_sub(1));
    doc.touch();
    true
}

/// Quick Fur: a hair particle system on each selected mesh.
fn quick_fur(doc: &mut Doc) -> bool {
    let sel = mesh_selection(doc);
    if sel.is_empty() {
        doc.report = "Select at least one mesh object".into();
        return true;
    }
    doc.push_undo("Quick Fur");
    for &i in &sel {
        let s = ParticleSettings { count: 1000, hair_length: 0.1, child_type: ph::ChildType::Interpolated, child_nbr: 10, rendered_child_count: 100, ..ParticleSettings::hair() };
        add_system(&mut doc.scene, i, s);
    }
    doc.report = format!("Added fur to {} object{}", sel.len(), if sel.len() == 1 { "" } else { "s" });
    doc.touch();
    true
}

/// Quick Smoke: the selected meshes become smoke Flow objects inside a
/// new Domain box around them, twice as tall to leave room to rise.
fn quick_smoke(doc: &mut Doc) -> bool {
    let sel = mesh_selection(doc);
    if sel.is_empty() {
        doc.report = "Select at least one mesh object".into();
        return true;
    }
    doc.push_undo("Quick Smoke");
    let (mut lo, mut hi) = (Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY), Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY));
    for &i in &sel {
        let m = doc.scene.world_matrix(i);
        if let Some(mesh) = doc.scene.evaluated_mesh(i) {
            for p in &mesh.positions {
                let w = m.transform_point(*p);
                lo = lo.min(w);
                hi = hi.max(w);
            }
        }
        let mut f = FluidSettings::new(FluidType::Flow);
        f.flow_type = ph::FlowType::Smoke;
        doc.scene.objects[i].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    }
    if !lo.x.is_finite() {
        (lo, hi) = (Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
    }
    // Blender: the domain is the selection's box, grown, with its top
    // raised so the smoke has room.
    let size = (hi - lo).max(Vec3::new(0.1, 0.1, 0.1));
    let lo = lo - size * 0.5;
    let hi = hi + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 2.0);
    let center = (lo + hi) * 0.5;
    let mesh = sex_3d::primitives::cube(2.0);
    let d = doc.scene.add_mesh_object("Smoke Domain", mesh);
    let o = &mut doc.scene.objects[d];
    o.transform.location = center;
    o.transform.scale = (hi - lo) * 0.5;
    let mut f = FluidSettings::new(FluidType::Domain);
    f.domain_type = DomainType::Gas;
    f.cache_frame_end = doc.scene.frame_end.min(f.cache_frame_end);
    o.modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    // Hidden in the viewport (the smoke preview draws its box instead of a
    // solid cube; Blender shows it as wire), still rendered.
    o.visible = false;
    doc.select_only(Some(d));
    doc.report = "Added a smoke domain".into();
    doc.touch();
    true
}

/// The fluid domains an operator acts on: the one named by `arg`
/// ("OBJECT" or a cache key), else the active object's.
fn fluid_keys(doc: &Doc, arg: &str) -> Vec<String> {
    if !arg.is_empty() {
        if let Some(i) = doc.scene.find_object(arg) {
            return ph::fluid::domains(&doc.scene).into_iter().filter(|d| d.0 == i).map(|d| ph::fluid::key(&doc.scene.objects[i].name, d.1)).collect();
        }
        return vec![arg.to_string()];
    }
    let Some(i) = doc.active() else { return Vec::new() };
    ph::fluid::domains(&doc.scene).into_iter().filter(|d| d.0 == i).map(|d| ph::fluid::key(&doc.scene.objects[i].name, d.1)).collect()
}

/// Fluid > Cache > Bake All (and Bake Data / Bake Mesh, which bake both).
fn fluid_bake(doc: &mut Doc, arg: &str) -> bool {
    let keys = fluid_keys(doc, arg);
    if keys.is_empty() {
        doc.report = "No fluid domain".into();
        return true;
    }
    bake(doc, keys, false)
}

/// Fluid > Cache > Free All (Free Data / Free Mesh).
fn fluid_free(doc: &mut Doc, arg: &str) -> bool {
    let keys = fluid_keys(doc, arg);
    if keys.is_empty() {
        doc.report = "No fluid domain".into();
        return true;
    }
    free(doc, keys, false)
}

/// Dynamic Paint > Add/Remove Canvas or Brush (`CANVAS`, `BRUSH`).
fn dpaint_toggle(doc: &mut Doc, arg: &str) -> bool {
    let Some(i) = doc.active() else { return true };
    let Some(slot) = modifier_slot(&doc.scene.objects[i], "DYNAMIC_PAINT") else {
        doc.report = "No Dynamic Paint modifier".into();
        return true;
    };
    let canvas = !arg.eq_ignore_ascii_case("BRUSH");
    doc.push_undo(if canvas { "Toggle Canvas" } else { "Toggle Brush" });
    if let Modifier::Physics(PhysicsModifier::DynamicPaint(s)) = &mut doc.scene.objects[i].modifiers[slot] {
        if canvas {
            s.use_canvas = !s.use_canvas;
            s.ui_type = ph::DpUiType::Canvas;
            s.frame_end = s.frame_end.min(doc.scene.frame_end.max(s.frame_start));
        } else {
            s.use_brush = !s.use_brush;
            s.ui_type = ph::DpUiType::Brush;
        }
    }
    doc.touch();
    true
}

fn selection_bounds(doc: &Doc, sel: &[usize]) -> (Vec3, Vec3) {
    let (mut lo, mut hi) = (Vec3::new(f32::INFINITY, f32::INFINITY, f32::INFINITY), Vec3::new(f32::NEG_INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY));
    for &i in sel {
        let m = doc.scene.world_matrix(i);
        if let Some(mesh) = doc.scene.evaluated_mesh(i) {
            for p in &mesh.positions {
                let w = m.transform_point(*p);
                lo = lo.min(w);
                hi = hi.max(w);
            }
        }
    }
    if !lo.x.is_finite() {
        (lo, hi) = (Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
    }
    (lo, hi)
}

/// Quick Liquid: the selected meshes become Geometry liquid flows, inside a new closed Liquid Domain with room around and below,
/// shaded smooth with a water-like glass material (Blender's operator).
fn quick_liquid(doc: &mut Doc) -> bool {
    let sel = mesh_selection(doc);
    if sel.is_empty() {
        doc.report = "Select at least one mesh object".into();
        return true;
    }
    doc.push_undo("Quick Liquid");
    let (lo, hi) = selection_bounds(doc, &sel);
    for &i in &sel {
        let mut f = FluidSettings::new(FluidType::Flow);
        f.flow_type = ph::FlowType::Liquid;
        f.flow_behavior = ph::FlowBehavior::Geometry;
        f.surface_distance = 0.0;
        let o = &mut doc.scene.objects[i];
        o.modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
        // Blender shows them as wire; here they are hidden (the liquid takes
        // their place from the start frame).
        o.visible = false;
    }
    let mesh = sex_3d::primitives::cube(2.0);
    let d = doc.scene.add_mesh_object("Liquid Domain", mesh);
    let center = (lo + hi) * 0.5 + Vec3::new(0.0, 0.0, -1.0);
    let half = (hi - lo) * 0.5 + Vec3::new(1.0, 1.0, 2.0);
    let mut f = FluidSettings::new(FluidType::Domain);
    f.domain_type = DomainType::Liquid;
    f.use_collision_border_front = true;
    f.use_collision_border_back = true;
    f.use_collision_border_right = true;
    f.use_collision_border_left = true;
    f.use_collision_border_top = true;
    f.use_collision_border_bottom = true;
    f.display_thickness = 0.02;
    f.cache_frame_end = doc.scene.frame_end.min(f.cache_frame_end);
    let mat = liquid_material(&mut doc.scene);
    let o = &mut doc.scene.objects[d];
    o.transform.location = center;
    o.transform.scale = half;
    o.modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    o.materials = vec![mat];
    if let ObjectData::Mesh(mi) = o.data {
        for face in &mut doc.scene.meshes[mi].faces {
            face.smooth = true;
        }
    }
    doc.select_only(Some(d));
    doc.report = "Added a liquid domain".into();
    doc.touch();
    true
}

/// "Liquid Domain Material": clear water (glass, IOR 1.33, a faint blue
/// absorption tint).
fn liquid_material(scene: &mut Scene) -> usize {
    let mut m = sex_3d::scene::Material::default();
    m.name = "Liquid Domain Material".into();
    m.base_color = [0.8, 0.9, 1.0, 1.0];
    m.transmission = 1.0;
    m.roughness = 0.0;
    m.metallic = 0.0;
    m.ior = 1.33;
    scene.materials.push(m);
    scene.materials.len() - 1
}

/// `--physics-demo`: boxes falling on a passive ground next to a cloth
/// draped over a sphere, a particle emitter and a smoke column, stepped
/// to frame 40 in the Physics tab.
pub fn demo(doc: &mut Doc) {
    if let Some(c) = doc.scene.find_object("Cube") {
        doc.select_only(Some(c));
        doc.run(&Op::new("object.delete"));
    }
    let scene = &mut doc.scene;
    let ground = scene.add_mesh_object("Ground", sex_3d::primitives::plane(14.0));
    scene.objects[ground].physics.rigid_body = Some(RigidBody { kind: RigidBodyType::Passive, collision_shape: CollisionShape::Box, ..Default::default() });
    scene.objects[ground].modifiers.push(Modifier::Physics(PhysicsModifier::Collision(Default::default())));
    ensure_world(scene);
    for k in 0..5 {
        let b = scene.add_mesh_object("Box", sex_3d::primitives::cube(1.0));
        let o = &mut scene.objects[b];
        o.transform.location = Vec3::new(-3.0 + (k % 2) as f32 * 0.3, -1.0, 0.6 + k as f32 * 1.4);
        o.transform.rotation = sex_3d::math::Quat::from_axis_angle(Vec3::Z, k as f32 * 0.4);
        o.physics.rigid_body = Some(RigidBody { collision_shape: CollisionShape::Box, ..Default::default() });
    }
    let ball = scene.add_mesh_object("Ball", sex_3d::primitives::uv_sphere(24, 12, 1.0));
    scene.objects[ball].transform.location = Vec3::new(2.0, 0.0, 1.0);
    scene.objects[ball].modifiers.push(Modifier::Physics(PhysicsModifier::Collision(Default::default())));
    let cloth = scene.add_mesh_object("Cloth", sex_3d::primitives::grid(24, 24, 3.0));
    scene.objects[cloth].transform.location = Vec3::new(2.0, 0.0, 2.6);
    scene.objects[cloth].modifiers.push(Modifier::Physics(PhysicsModifier::with_defaults("cloth").unwrap()));
    let em = scene.add_mesh_object("Emitter", sex_3d::primitives::plane(1.0));
    scene.objects[em].transform.location = Vec3::new(3.5, -2.5, 2.5);
    let s = ParticleSettings { count: 400, frame_end: 40.0, lifetime: 60.0, normal_factor: 3.0, display_method: ph::DisplayMethod::Circle, ..Default::default() };
    add_system(scene, em, s);
    let fire = scene.add_mesh_object("Smoke Source", sex_3d::primitives::ico_sphere(1, 0.3));
    scene.objects[fire].transform.location = Vec3::new(-0.5, 3.5, 0.4);
    doc.select_only(Some(fire));
    quick_smoke(doc);
    let f = doc.scene.objects.iter().position(|o| o.name == "Smoke Domain");
    if let Some(d) = f {
        if let Some(Modifier::Physics(PhysicsModifier::Fluid(s))) = doc.scene.objects[d].modifiers.last_mut() {
            s.resolution_max = 24;
        }
    }
    doc.scene.frame_end = 80;
    if let Some(b) = doc.scene.find_object("Box.002") {
        doc.select_only(Some(b));
    }
    doc.frame = 40;
    doc.scene.frame_current = 40;
    ph::update(&doc.scene);
    doc.sim.last = None;
    doc.touch();
}

/// `--liquid-demo`: a dam break in a closed Quick Liquid domain, the water
/// column hitting an obstacle block while a tap pours in from above and a
/// drain takes water away; a red ball and a Dynamic Paint canvas ripple
/// next to it. Stepped to frame 24 in the Physics tab, the domain active.
pub fn liquid_demo(doc: &mut Doc) {
    if let Some(c) = doc.scene.find_object("Cube") {
        doc.select_only(Some(c));
        doc.run(&Op::new("object.delete"));
    }
    let scene = &mut doc.scene;
    let floor = scene.add_mesh_object("Floor", sex_3d::primitives::plane(14.0));
    scene.objects[floor].transform.location = Vec3::new(0.0, 0.0, -0.02);
    let col = scene.add_mesh_object("Water", sex_3d::primitives::cube(1.0));
    scene.objects[col].transform.location = Vec3::new(-1.5, 0.0, 0.75);
    scene.objects[col].transform.scale = Vec3::new(1.0, 2.0, 1.5);
    doc.select_only(Some(col));
    quick_liquid(doc);
    let scene = &mut doc.scene;
    let d = scene.find_object("Liquid Domain").expect("domain");
    // The domain: x ∈ [-2, 2], y ∈ [-1, 1], z ∈ [0, 2].
    scene.objects[d].transform.location = Vec3::new(0.0, 0.0, 1.0);
    scene.objects[d].transform.scale = Vec3::new(2.0, 1.0, 1.0);
    if let Some(Modifier::Physics(PhysicsModifier::Fluid(s))) = scene.objects[d].modifiers.last_mut() {
        s.resolution_max = 48;
        s.cache_frame_end = 80;
    }
    let block = scene.add_mesh_object("Obstacle", sex_3d::primitives::cube(1.0));
    scene.objects[block].transform.location = Vec3::new(0.6, 0.0, 0.3);
    scene.objects[block].transform.scale = Vec3::new(0.4, 0.8, 0.6);
    let mut e = FluidSettings::new(FluidType::Effector);
    e.effector_type = ph::EffectorType::Collision;
    scene.objects[block].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(e))));
    let tap = scene.add_mesh_object("Tap", sex_3d::primitives::ico_sphere(2, 0.18));
    scene.objects[tap].transform.location = Vec3::new(1.4, 0.0, 1.6);
    let mut f = FluidSettings::new(FluidType::Flow);
    f.flow_type = ph::FlowType::Liquid;
    f.flow_behavior = ph::FlowBehavior::Inflow;
    f.surface_distance = 0.0;
    f.use_initial_velocity = true;
    f.velocity_coord = [-1.5, 0.0, -1.0];
    scene.objects[tap].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    let drain = scene.add_mesh_object("Drain", sex_3d::primitives::cube(1.0));
    scene.objects[drain].transform.location = Vec3::new(1.75, 0.0, 0.08);
    scene.objects[drain].transform.scale = Vec3::new(0.5, 2.0, 0.16);
    let mut f = FluidSettings::new(FluidType::Flow);
    f.flow_type = ph::FlowType::Liquid;
    f.flow_behavior = ph::FlowBehavior::Outflow;
    f.surface_distance = 0.0;
    scene.objects[drain].modifiers.push(Modifier::Physics(PhysicsModifier::Fluid(Box::new(f))));
    scene.objects[drain].visible = false;
    // Dynamic Paint: a ball rippling a pool surface.
    let canvas = scene.add_mesh_object("Pool", sex_3d::primitives::grid(40, 40, 2.4));
    scene.objects[canvas].transform.location = Vec3::new(0.0, 3.2, 0.3);
    let dp = ph::DynamicPaintSettings { use_canvas: true, surface_type: ph::DpSurfaceType::Wave, frame_end: 80, ..Default::default() };
    scene.objects[canvas].modifiers.push(Modifier::Physics(PhysicsModifier::DynamicPaint(Box::new(dp))));
    let ball = scene.add_mesh_object("Ball", sex_3d::primitives::uv_sphere(24, 12, 0.3));
    scene.objects[ball].transform.location = Vec3::new(0.0, 3.2, 0.4);
    let red = scene.add_material(sex_3d::scene::Material { name: "Red".into(), base_color: [0.8, 0.1, 0.05, 1.0], ..Default::default() });
    scene.objects[ball].materials.push(red);
    let br = ph::DynamicPaintSettings { ui_type: ph::DpUiType::Brush, use_brush: true, ..Default::default() };
    scene.objects[ball].modifiers.push(Modifier::Physics(PhysicsModifier::DynamicPaint(Box::new(br))));
    doc.scene.frame_end = 80;
    doc.select_only(Some(d));
    doc.frame = 24;
    doc.scene.frame_current = 24;
    ph::update(&doc.scene);
    doc.sim.last = None;
    doc.report = "Liquid demo".into();
    doc.touch();
}
