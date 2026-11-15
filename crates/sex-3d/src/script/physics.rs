// SPDX-License-Identifier: GPL-3.0-only
//! polyops commands for physics settings. Every option is a property id
//! of [`crate::physics::props`] (Blender's Python names), written only when
//! it differs from the default:
//!
//! ```text
//! physics_scene use_gravity=1 gravity=0,0,-9.81
//! rigidbody_world substeps_per_frame=20 cache_frame_end=120
//! rigidbody Cube type=passive collision_shape=box friction=0.2
//! rigidbody_constraint Empty type=hinge object1=A object2=B
//! field Empty type=wind strength=2
//! modifier_add Cube Cloth quality=10 vertex_group_mass=Pin
//! modifier_add Cube ParticleSystem name=Sparks settings_name=Fire count=500
//! ```
//!
//! `rigidbody_world_remove`, `rigidbody_remove OBJ`,
//! `rigidbody_constraint_remove OBJ` and `field_remove OBJ` delete them.
//! Caches are runtime data and are not written (a Disk Cache bake lives in
//! the `cache` folder next to the file).
use super::*;
use crate::physics::props::{self, PropEnum, Props};
use crate::physics::{FieldSettings, FieldType, ParticleSystem, PhysicsModifier, RigidBody, RigidBodyConstraint, RigidBodyWorld};

/// Commands handled here.
pub(super) const COMMANDS: &[&str] = &[
    "physics_scene",
    "rigidbody_world",
    "rigidbody_world_remove",
    "rigidbody",
    "rigidbody_remove",
    "rigidbody_constraint",
    "rigidbody_constraint_remove",
    "field",
    "field_remove",
];

/// Set every unused option of `cmd` that names a property of `p`. Options
/// are applied in passes so that type-dependent properties (shown only for
/// some types) may come before or after the type.
fn apply(cmd: &mut Cmd, p: &mut dyn Props) -> Res<()> {
    loop {
        let mut progress = false;
        for k in 0..cmd.opts.len() {
            if cmd.opts[k].2 {
                continue;
            }
            let (key, text) = (cmd.opts[k].0.clone(), cmd.opts[k].1.text.clone());
            if props::set_text(p, &key, &text)? {
                cmd.opts[k].2 = true;
                progress = true;
            }
        }
        if !progress {
            return Ok(());
        }
    }
}

/// Options of a physics modifier (`modifier_add` / `modifier_set`).
pub(super) fn modifier_opts(cmd: &mut Cmd, m: &mut Modifier) -> Res<bool> {
    let Modifier::Physics(p) = m else { return Ok(false) };
    if let PhysicsModifier::ParticleSystem(sys) = p {
        if let Some(t) = cmd.opt("name") {
            sys.name = t.text;
        }
        if let Some(t) = cmd.opt("settings_name") {
            sys.settings.name = t.text;
        }
    }
    apply(cmd, p.props())?;
    Ok(true)
}

/// ` key=value` options of a physics modifier.
pub(super) fn modifier_to_options(m: &Modifier) -> String {
    let Modifier::Physics(p) = m else { return String::new() };
    let mut out = String::new();
    if let PhysicsModifier::ParticleSystem(sys) = p {
        let d = ParticleSystem::default();
        if sys.name != d.name {
            out += &format!(" name={}", quote(&sys.name));
        }
        if sys.settings.name != d.settings.name {
            out += &format!(" settings_name={}", quote(&sys.settings.name));
        }
    }
    let mut p = p.clone();
    let mut d = PhysicsModifier::with_defaults(&p.name().to_ascii_lowercase()).expect("physics modifier");
    out + &props::diff_options(p.props(), d.props())
}

/// Scene and object physics lines for the scene text.
pub(super) fn write_scene(o: &mut String, scene: &Scene) {
    let ph = &scene.physics;
    let mut g = ph.gravity.clone();
    let mut dg = crate::physics::SceneGravity::default();
    let opts = props::diff_options(&mut g, &mut dg);
    if !opts.is_empty() {
        let _ = writeln!(o, "physics_scene{opts}");
    }
    if let Some(w) = &ph.rigidbody_world {
        let _ = writeln!(o, "rigidbody_world{}", props::diff_options(&mut w.clone(), &mut RigidBodyWorld::default()));
    }
}

pub(super) fn write_object(o: &mut String, ob: &Object) {
    let name = quote(&ob.name);
    let ph = &ob.physics;
    if let Some(rb) = &ph.rigid_body {
        let _ = writeln!(o, "rigidbody {name}{}", props::diff_options(&mut rb.clone(), &mut RigidBody::default()));
    }
    if let Some(c) = &ph.rigid_body_constraint {
        let _ = writeln!(o, "rigidbody_constraint {name}{}", props::diff_options(&mut c.clone(), &mut RigidBodyConstraint::default()));
    }
    if let Some(f) = &ph.field {
        // The type always, then what differs from that type's defaults.
        let mut d = FieldSettings::new(f.kind);
        let _ = writeln!(o, "field {name} type={}{}", f.kind.ident().to_ascii_lowercase(), props::diff_options(&mut f.clone(), &mut d));
    }
}

impl Session {
    pub(super) fn physics_command(&mut self, scene: &mut Scene, cmd: &mut Cmd) -> Res<Option<String>> {
        match cmd.name.as_str() {
            "physics_scene" => {
                cmd.max_args(0)?;
                apply(cmd, &mut scene.physics.gravity)?;
            }
            "rigidbody_world" => {
                cmd.max_args(0)?;
                let w = scene.physics.rigidbody_world.get_or_insert_with(RigidBodyWorld::default);
                apply(cmd, w)?;
            }
            "rigidbody_world_remove" => {
                cmd.max_args(0)?;
                scene.physics.rigidbody_world = None;
            }
            "rigidbody" | "rigidbody_constraint" | "field" => {
                cmd.max_args(1)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let ph = &mut scene.objects[i].physics;
                match cmd.name.as_str() {
                    "rigidbody" => apply(cmd, ph.rigid_body.get_or_insert_with(RigidBody::default))?,
                    "rigidbody_constraint" => apply(cmd, ph.rigid_body_constraint.get_or_insert_with(RigidBodyConstraint::default))?,
                    _ => {
                        // A new field starts from its type's defaults.
                        if ph.field.is_none() {
                            let kind = match cmd.opt("type") {
                                Some(t) => FieldType::parse(&t.text).ok_or_else(|| format!("type: '{}' is not a field type", t.text))?,
                                None => FieldType::Force,
                            };
                            ph.field = Some(FieldSettings::new(kind));
                        }
                        apply(cmd, ph.field.as_mut().expect("field"))?;
                    }
                }
            }
            "rigidbody_remove" | "rigidbody_constraint_remove" | "field_remove" => {
                cmd.max_args(1)?;
                let i = self.object(scene, cmd.arg(0, "an object")?)?;
                let ph = &mut scene.objects[i].physics;
                match cmd.name.as_str() {
                    "rigidbody_remove" => ph.rigid_body = None,
                    "rigidbody_constraint_remove" => ph.rigid_body_constraint = None,
                    _ => ph.field = None,
                }
            }
            other => return Err(format!("unknown command '{other}'")),
        }
        Ok(None)
    }
}
