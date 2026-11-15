// SPDX-License-Identifier: GPL-3.0-only
//! Object Properties (name, Transform, Delta Transform, Relations,
//! Collections, Instancing, Visibility, Viewport Display) and the Modifiers
//! tab with Blender's modifier cards.
use super::icons::{self, Icon};
use super::layout::{L, GAP, ROW};
use super::Ctx;
use crate::doc::Doc;
use crate::draw::{Align, Rect};
use crate::settings::RotationMode;
use crate::theme as t;
use crate::ui::{MenuItem, NumOpts, Op};
use sex_3d::modifiers::{BooleanOp, CastType, DecimateMode, Modifier, RemeshMode, ShrinkwrapMethod, SimpleDeformMode};
use sex_3d::nodes::{SocketType, Value};
use sex_3d::physics::PhysicsModifier;
use sex_3d::scene::{ObjectData, RayVisibility, Transform};
use sex_3d::script::quat_to_euler_xyz;
use sex_3d::{Quat, Vec3};

fn inactive<R>(l: &mut L, f: impl FnOnce(&mut L) -> R) {
    let was = l.inactive;
    l.inactive = true;
    let _ = f(l);
    l.inactive = was;
}

const M: NumOpts<'static> = NumOpts { step: 0.1, unit: "m", min: f32::NEG_INFINITY, max: f32::INFINITY, slider: false, int: false };
const PLAIN: NumOpts<'static> = NumOpts { step: 0.1, unit: "", min: f32::NEG_INFINITY, max: f32::INFINITY, slider: false, int: false };

/// Rename an object the way Blender does (unique ".001" suffixes), carrying
/// its settings along.
pub fn rename_object(d: &mut Doc, o: usize, want: &str) {
    let want = want.trim();
    if want.is_empty() || d.scene.objects[o].name == want {
        return;
    }
    d.push_undo("Rename");
    let n = sex_3d::scene::unique_name(want, |n| d.scene.objects.iter().enumerate().any(|(i, x)| i != o && x.name == n));
    let old = core::mem::replace(&mut d.scene.objects[o].name, n.clone());
    d.settings.rename_object(&old, &n);
    d.anim.rename_object(&old, &n);
    d.rig.rename_object(&old, &n);
    // Constraint targets and drivers name objects too.
    for ob in &mut d.scene.objects {
        ob.constraints.iter_mut().for_each(|c| c.rename_object_ref(&old, &n));
        for pb in &mut ob.pose {
            pb.constraints.iter_mut().for_each(|c| c.rename_object_ref(&old, &n));
        }
    }
    // Boolean operands and Shrinkwrap targets refer to objects by name.
    for ob in &mut d.scene.objects {
        ob.modifiers.iter_mut().for_each(|m| m.rename_object_ref(&old, &n));
    }
    for p in &mut d.settings.parked {
        p.modifier.rename_object_ref(&old, &n);
        p.stack.iter_mut().for_each(|m| m.rename_object_ref(&old, &n));
    }
    d.touch();
}

/// Parent `o` to `p` keeping its world transform (Blender's "Object (Keep
/// Transform)"); `None` clears the parent the same way.
pub fn set_parent(d: &mut Doc, o: usize, p: Option<usize>) {
    if let Some(p) = p {
        // No cycles: `p` must not be `o` or below it.
        let mut c = Some(p);
        let mut guard = 0;
        while let Some(ci) = c {
            if ci == o {
                d.report = "Loop in parents".into();
                return;
            }
            c = d.scene.objects[ci].parent;
            guard += 1;
            if guard > d.scene.objects.len() {
                break;
            }
        }
    }
    if d.scene.objects[o].parent == p {
        return;
    }
    d.push_undo(if p.is_some() { "Make Parent" } else { "Clear Parent" });
    let world = d.scene.world_matrix(o);
    let local = match p {
        Some(p) => d.scene.world_matrix(p).inverse() * world,
        None => world,
    };
    d.scene.objects[o].transform = Transform::from_matrix(&local);
    d.scene.objects[o].parent = p;
    d.touch();
}

pub fn object(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some(o) = c.obj else {
        l.note("No active object");
        return;
    };
    // Name with the object icon (Blender's ID block).
    let name = d.scene.objects[o].name.clone();
    if let Some(n) = l.id_block(super::object_icon(&d.scene.objects[o].data), &name, Some("ob_name")) {
        rename_object(d, o, &n);
    }
    let name = d.scene.objects[o].name.clone();
    // Lengths show metres unless the unit system is None.
    let lu = if d.settings.unit_system == crate::settings::UnitSystem::None { "" } else { "m" };
    l.panel(d, "object.transform", "Transform", true, &mut |l, d| {
        let tr = d.scene.objects[o].transform;
        for (i, lab) in ["Location X", "Y", "Z"].iter().enumerate() {
            let mut v = tr.location.to_array()[i];
            l.bind(d, &format!("loc{i}"), &name, "location", i);
            if l.number(lab, &format!("loc{i}"), &mut v, NumOpts { unit: lu, ..M }) {
                l.commit(d, "Location");
                let mut a = tr.location.to_array();
                a[i] = v;
                d.scene.objects[o].transform.location = Vec3::from_array(a);
            }
        }
        l.space(4);
        let mode = d.settings.rotation_mode(&name);
        let deg = d.settings.degrees;
        let (unit, k) = if deg { ("°", 180.0 / core::f32::consts::PI) } else { ("", 1.0) };
        let ang = NumOpts { step: if deg { 1.0 } else { 0.02 }, unit, ..PLAIN };
        match mode {
            RotationMode::EulerXyz => {
                let e = quat_to_euler_xyz(tr.rotation);
                for (i, lab) in ["Rotation X", "Y", "Z"].iter().enumerate() {
                    let mut v = e[i] * k;
                    l.bind(d, &format!("rot{i}"), &name, "rotation_euler", i);
                    if l.number(lab, &format!("rot{i}"), &mut v, ang) {
                        l.commit(d, "Rotation");
                        let mut e2 = e;
                        e2[i] = v / k;
                        d.scene.objects[o].transform.rotation = Quat::from_euler_xyz(e2[0], e2[1], e2[2]);
                    }
                }
            }
            RotationMode::Quaternion => {
                let q = tr.rotation;
                let mut a = [q.w, q.x, q.y, q.z];
                for (i, lab) in ["Rotation W", "X", "Y", "Z"].iter().enumerate() {
                    let mut v = a[i];
                    l.bind(d, &format!("quat{i}"), &name, "rotation_quaternion", i);
                    if l.number(lab, &format!("quat{i}"), &mut v, NumOpts { step: 0.01, ..PLAIN }) {
                        l.commit(d, "Rotation");
                        a[i] = v;
                        d.scene.objects[o].transform.rotation = Quat { w: a[0], x: a[1], y: a[2], z: a[3] }.normalize();
                    }
                }
            }
            RotationMode::AxisAngle => {
                let q = tr.rotation.normalize();
                let angle = 2.0 * q.w.clamp(-1.0, 1.0).acos();
                let s = (1.0 - q.w * q.w).max(0.0).sqrt();
                let axis = if s < 1e-5 { Vec3::Y } else { Vec3::new(q.x / s, q.y / s, q.z / s) };
                let mut a = [angle * k, axis.x, axis.y, axis.z];
                for (i, lab) in ["Rotation W", "X", "Y", "Z"].iter().enumerate() {
                    let mut v = a[i];
                    let o2 = if i == 0 { ang } else { NumOpts { step: 0.01, ..PLAIN } };
                    l.bind(d, &format!("aa{i}"), &name, "rotation_axis_angle", i);
                    if l.number(lab, &format!("aa{i}"), &mut v, o2) {
                        l.commit(d, "Rotation");
                        a[i] = v;
                        let ax = Vec3::new(a[1], a[2], a[3]);
                        let ax = if ax.length() < 1e-6 { Vec3::Y } else { ax };
                        d.scene.objects[o].transform.rotation = Quat::from_axis_angle(ax, a[0] / k);
                    }
                }
            }
        }
        l.enumeration("Mode", "rotation_mode", mode.label(), &["Quaternion (WXYZ)", "XYZ Euler", "XZY Euler", "YXZ Euler", "YZX Euler", "ZXY Euler", "ZYX Euler", "Axis Angle"]);
        l.space(4);
        for (i, lab) in ["Scale X", "Y", "Z"].iter().enumerate() {
            let mut v = tr.scale.to_array()[i];
            l.bind(d, &format!("scale{i}"), &name, "scale", i);
            if l.number(lab, &format!("scale{i}"), &mut v, NumOpts { step: 0.05, ..PLAIN }) {
                l.commit(d, "Scale");
                let mut a = tr.scale.to_array();
                a[i] = v;
                d.scene.objects[o].transform.scale = Vec3::from_array(a);
            }
        }
    });
    l.panel(d, "object.delta", "Delta Transform", false, &mut |l, _| {
        inactive(l, |l| {
            let mut z = 0.0;
            let mut one = 1.0;
            for (i, lab) in ["Delta Location X", "Y", "Z"].iter().enumerate() {
                l.number(lab, &format!("dloc{i}"), &mut z, M);
            }
            for (i, lab) in ["Delta Rotation X", "Y", "Z"].iter().enumerate() {
                l.number(lab, &format!("drot{i}"), &mut z, NumOpts { unit: "°", ..PLAIN });
            }
            for (i, lab) in ["Delta Scale X", "Y", "Z"].iter().enumerate() {
                l.number(lab, &format!("dscale{i}"), &mut one, PLAIN);
            }
        });
    });
    l.panel(d, "object.relations", "Relations", false, &mut |l, d| {
        let cur = d.scene.objects[o].parent.map(|p| d.scene.objects[p].name.clone()).unwrap_or_default();
        let mut items: Vec<String> = vec![String::new()];
        items.extend(d.scene.objects.iter().enumerate().filter(|(i, _)| *i != o).map(|(_, x)| x.name.clone()));
        let refs: Vec<&str> = items.iter().map(|s| if s.is_empty() { "(None)" } else { s.as_str() }).collect();
        l.enumeration("Parent", "parent", &cur, &refs);
        l.info("Parent Type", "Object");
        inactive(l, |l| {
            let mut v = 0;
            l.int("Pass Index", "pass_index", &mut v, 0, 32767);
        });
    });
    l.panel(d, "object.collections", "Collections", false, &mut |l, d| {
        let ci = d.scene.objects[o].collection;
        let cname = d.scene.collections.get(ci).map(|c| c.name.clone()).unwrap_or_default();
        if l.band(ROW + 8) {
            let (x, w) = l.inner();
            let bx = Rect::new(x, l.y, w, ROW + 8);
            l.ui.p.round_rect(bx, 4, 0x353535);
            icons::draw(l.ui.p, Icon::Collection, x + 12, l.y + (ROW + 8) / 2);
            l.ui.p.label(Rect::new(x + 24, l.y, w - 30, ROW + 8), &cname, t::TEXT, Align::Left);
            l.y += ROW + 8 + GAP;
        }
        let names: Vec<String> = d.scene.collections.iter().map(|c| c.name.clone()).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        l.enumeration("", "add_collection", "Move to Collection", &refs);
    });
    l.panel(d, "object.instancing", "Instancing", false, &mut |l, _| {
        inactive(l, |l| {
            l.expand("", &["None", "Vertices", "Faces"], 0);
        });
    });
    l.panel(d, "object.motion_paths", "Motion Paths", false, &mut |l, _| l.note("Not available yet"));
    l.panel(d, "object.visibility", "Visibility", false, &mut |l, d| {
        inactive(l, |l| {
            let mut t = true;
            l.check("", "Selectable", &mut t);
        });
        let mut v = d.scene.objects[o].visible;
        if l.check("Show In", "Viewports", &mut v) {
            l.commit(d, "Show in Viewports");
            d.scene.objects[o].visible = v;
        }
        let mut r = !d.settings.hide_render.contains(&name);
        if l.check("", "Renders", &mut r) {
            l.commit(d, "Show in Renders");
            if r {
                d.settings.hide_render.remove(&name);
            } else {
                d.settings.hide_render.insert(name.clone());
            }
        }
        l.panel(d, "object.visibility.rays", "Ray Visibility", true, &mut |l, d| {
            let vis = d.scene.objects[o].visibility;
            let rows: [(&str, &str, fn(&mut RayVisibility) -> &mut bool); 6] = [
                ("Ray Visibility", "Camera", |v| &mut v.camera),
                ("", "Diffuse", |v| &mut v.diffuse),
                ("", "Glossy", |v| &mut v.glossy),
                ("", "Transmission", |v| &mut v.transmission),
                ("", "Volume Scatter", |v| &mut v.volume_scatter),
                ("", "Shadow", |v| &mut v.shadow),
            ];
            for (heading, label, field) in rows {
                let mut nv = vis;
                let mut b = *field(&mut nv);
                if l.check(heading, label, &mut b) {
                    *field(&mut nv) = b;
                    l.commit(d, &format!("{label} Visibility"));
                    d.scene.objects[o].visibility = nv;
                }
            }
        });
        inactive(l, |l| {
            let mut f = false;
            l.check("Mask", "Shadow Catcher", &mut f);
        });
        let mut h = d.scene.objects[o].holdout;
        if l.check("", "Holdout", &mut h) {
            l.commit(d, "Holdout");
            d.scene.objects[o].holdout = h;
        }
    });
    l.panel(d, "object.render", "Render", false, &mut |l, d| {
        let mut v = d.scene.objects[o].pass_index;
        if l.uint("Pass Index", "ob_pass_index", &mut v, 0, 32767) {
            l.commit(d, "Pass Index");
            d.scene.objects[o].pass_index = v;
        }
    });
    l.panel(d, "object.viewport_display", "Viewport Display", false, &mut |l, _| {
        inactive(l, |l| {
            let mut f = false;
            l.check("Show", "Name", &mut f);
            l.check("", "Axes", &mut f);
            l.check("", "Wireframe", &mut f);
            l.check("", "All Edges", &mut f);
            l.check("", "Texture Space", &mut f);
            let mut tt = true;
            l.check("", "Shadow", &mut tt);
            l.check("", "In Front", &mut f);
            l.enumeration("Display As", "display_as", "Textured", &[]);
            l.color("Color", [1.0, 1.0, 1.0]);
            l.check("", "Bounds", &mut f);
        });
    });
    l.panel(d, "object.line_art", "Line Art", false, &mut |l, _| inactive(l, |l| l.enumeration("Usage", "la", "Inherit", &[])));
    l.panel(d, "object.custom_props", "Custom Properties", false, &mut |l, _| l.note("Not available yet"));
}

/// Blender's Add Modifier menu, by category. Ours are live, the rest report
/// that they are not available yet.
pub fn add_modifier_menu() -> Vec<MenuItem> {
    let live = |n: &str, a: &str| MenuItem::op(n, "", Op::with("object.modifier_add", a));
    let na = |n: &str| MenuItem::op(n, "", Op::with("properties.unavailable", &format!("{n} modifier")));
    let cat = |items: &[(&str, &str)]| items.iter().map(|(n, a)| if a.is_empty() { na(n) } else { live(n, a) }).collect::<Vec<_>>();
    vec![
        MenuItem::Sub(
            "Edit".into(),
            cat(&[("Data Transfer", ""), ("Mesh Cache", ""), ("Mesh Sequence Cache", ""), ("UV Project", ""), ("UV Warp", ""), ("Vertex Weight Edit", ""), ("Vertex Weight Mix", ""), ("Vertex Weight Proximity", "")]),
        ),
        MenuItem::Sub(
            "Generate".into(),
            cat(&[
                ("Array", "ARRAY"),
                ("Bevel", "BEVEL"),
                ("Boolean", "BOOLEAN"),
                ("Build", "BUILD"),
                ("Decimate", "DECIMATE"),
                ("Edge Split", "EDGE_SPLIT"),
                ("Geometry Nodes", "NODES"),
                ("Mask", ""),
                ("Mirror", "MIRROR"),
                ("Multiresolution", ""),
                ("Remesh", "REMESH"),
                ("Screw", "SCREW"),
                ("Skin", ""),
                ("Solidify", "SOLIDIFY"),
                ("Subdivision Surface", "SUBSURF"),
                ("Triangulate", "TRIANGULATE"),
                ("Volume to Mesh", ""),
                ("Weld", "WELD"),
                ("Wireframe", "WIREFRAME"),
            ]),
        ),
        MenuItem::Sub(
            "Deform".into(),
            cat(&[
                ("Armature", ""),
                ("Cast", "CAST"),
                ("Curve", ""),
                ("Displace", "DISPLACE"),
                ("Hook", ""),
                ("Laplacian Deform", ""),
                ("Lattice", ""),
                ("Mesh Deform", ""),
                ("Shrinkwrap", "SHRINKWRAP"),
                ("Simple Deform", "SIMPLE_DEFORM"),
                ("Smooth", "SMOOTH"),
                ("Smooth Corrective", "CORRECTIVE_SMOOTH"),
                ("Smooth Laplacian", "LAPLACIANSMOOTH"),
                ("Surface Deform", ""),
                ("Warp", ""),
                ("Wave", "WAVE"),
            ]),
        ),
        MenuItem::Sub("Normals".into(), cat(&[("Normal Edit", ""), ("Weighted Normal", "WEIGHTED_NORMAL"), ("Smooth by Angle", "")])),
        MenuItem::Sub(
            "Physics".into(),
            cat(&[("Cloth", ""), ("Collision", ""), ("Dynamic Paint", ""), ("Explode", ""), ("Fluid", ""), ("Ocean", ""), ("Particle Instance", ""), ("Particle System", ""), ("Soft Body", "")]),
        ),
    ]
}

fn label_of(m: &Modifier) -> &'static str {
    match m {
        Modifier::Subdivision { .. } => "Subdivision",
        other => other.name(),
    }
}

/// Write a whole shown stack back: enabled entries live, disabled parked.
fn write_stack(d: &mut Doc, o: usize, full: Vec<(Modifier, bool)>) {
    let name = d.scene.objects[o].name.clone();
    d.settings.parked.retain(|p| p.object != name);
    let live: Vec<Modifier> = full.iter().filter(|e| e.1).map(|e| e.0.clone()).collect();
    for (i, (m, on)) in full.into_iter().enumerate() {
        if !on {
            d.settings.parked.push(crate::settings::ParkedModifier { object: name.clone(), slot: i, modifier: m, stack: live.clone() });
        }
    }
    d.scene.objects[o].modifiers = live;
    d.touch();
}

/// Modifier operators on entry `i` of the shown stack.
pub fn modifier_op(d: &mut Doc, o: usize, op: &str, i: usize) {
    let mut full: Vec<(Modifier, bool)> = d.settings.shown_stack(&d.scene, o).into_iter().map(|(m, on, _)| (m, on)).collect();
    if i >= full.len() {
        return;
    }
    match op {
        "object.modifier_remove" => {
            d.push_undo("Delete Modifier");
            let m = full.remove(i);
            d.report = format!("Removed modifier: {}", label_of(&m.0));
        }
        "object.modifier_copy" => {
            d.push_undo("Duplicate Modifier");
            let m = full[i].clone();
            full.insert(i + 1, m);
        }
        "object.modifier_move_up" | "object.modifier_move_to_first" if i > 0 => {
            d.push_undo("Move Modifier");
            let m = full.remove(i);
            full.insert(if op.ends_with("first") { 0 } else { i - 1 }, m);
        }
        "object.modifier_move_down" | "object.modifier_move_to_last" if i + 1 < full.len() => {
            d.push_undo("Move Modifier");
            let m = full.remove(i);
            let to = if op.ends_with("last") { full.len() } else { i + 1 };
            full.insert(to, m);
        }
        "object.modifier_apply" => {
            let ObjectData::Mesh(mi) = d.scene.objects[o].data else { return };
            if !full[i].1 {
                d.report = "Modifier is disabled, skipping apply".into();
                return;
            }
            let users = d.scene.objects.iter().filter(|x| matches!(x.data, ObjectData::Mesh(m) if m == mi)).count();
            if users > 1 {
                d.report = "Modifiers cannot be applied to multi-user data".into();
                return;
            }
            d.push_undo("Apply Modifier");
            let first = full[..i].iter().all(|e| !e.1);
            let (m, _) = full.remove(i);
            d.scene.meshes[mi] = d.scene.apply_modifiers(o, &d.scene.meshes[mi], std::slice::from_ref(&m));
            d.report = if first { format!("Applied modifier: {}", label_of(&m)) } else { "Applied modifier was not first, result may not be as expected".into() };
        }
        _ => return,
    }
    write_stack(d, o, full);
}

/// The entry shown for "no object" in object fields.
pub const NO_OBJECT: &str = "(None)";

/// Set the object field of entry `si` of the shown stack (Boolean's operand,
/// Shrinkwrap's target).
pub fn set_modifier_object(d: &mut Doc, o: usize, si: usize, name: &str) {
    let shown = d.settings.shown_stack(&d.scene, o);
    let Some((m, true, li)) = shown.get(si).cloned() else { return };
    let mut nm = m.clone();
    match &mut nm {
        Modifier::Boolean { object: s, .. } | Modifier::Shrinkwrap { target: s, .. } => *s = name.to_string(),
        Modifier::Nodes { group, .. } => *group = name.to_string(),
        _ => return,
    }
    if nm != m {
        d.push_undo("Modifier Property");
        d.scene.objects[o].modifiers[li] = nm;
        d.touch();
    }
}

/// A Nodes modifier's inputs: the group interface's non-geometry inputs,
/// each showing the modifier's value or the interface default.
fn node_inputs(l: &mut L, d: &Doc, key: &str, group: &str, inputs: &mut Vec<(String, Value)>) -> bool {
    let Some(g) = d.scene.node_groups.iter().find(|g| g.name == group) else { return false };
    let mut ch = false;
    for (name, ty, default) in sex_3d::geometry::eval::modifier_interface(g) {
        let cur = inputs.iter().find(|(n, _)| *n == name).map_or(default, |x| x.1.clone());
        let k = format!("{key}in/{name}");
        let new = match ty {
            SocketType::Float => {
                let mut v = cur.as_float();
                l.number(&name, &k, &mut v, PLAIN).then_some(Value::Float(v))
            }
            SocketType::Int => {
                let mut v = match cur {
                    Value::Int(i) => i as i64,
                    ref other => other.as_float() as i64,
                };
                l.int(&name, &k, &mut v, i32::MIN as i64, i32::MAX as i64).then_some(Value::Int(v as i32))
            }
            SocketType::Bool => {
                let mut v = matches!(cur, Value::Bool(true));
                l.check("", &name, &mut v).then_some(Value::Bool(v))
            }
            SocketType::Vector | SocketType::Rotation => {
                let mut v = cur.as_vector();
                let mut changed = false;
                for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    let label = if i == 0 { format!("{name} {axis}") } else { axis.to_string() };
                    changed |= l.number(&label, &format!("{k}/{i}"), &mut v[i], PLAIN);
                }
                changed.then_some(Value::Vector(v))
            }
            SocketType::String => {
                let mut v = match cur {
                    Value::String(ref s) => s.clone(),
                    _ => String::new(),
                };
                l.text(&name, &k, &mut v).then_some(Value::String(v))
            }
            SocketType::Color => {
                let c = cur.as_color();
                l.info(&name, &format!("{:.3}, {:.3}, {:.3}, {:.3}", c[0], c[1], c[2], c[3]));
                None
            }
            _ => {
                l.info(&name, "Not editable here");
                None
            }
        };
        if let Some(v) = new {
            match inputs.iter_mut().find(|(n, _)| *n == name) {
                Some(slot) => slot.1 = v,
                None => inputs.push((name, v)),
            }
            ch = true;
        }
    }
    ch
}

/// A row of joined X / Y / Z toggles, like Mirror's axes.
fn toggles(l: &mut L, label: &str, v: &mut [bool; 3]) -> bool {
    let mut ch = false;
    if l.band(ROW) {
        let (lr, fr) = {
            let (ix, iw) = l.inner();
            let lw = (iw as f32 * 0.4) as i32;
            (Rect::new(ix, l.y, lw - 8, ROW), Rect::new(ix + lw, l.y, iw - lw, ROW))
        };
        l.ui.p.label(lr, label, t::TEXT, Align::Right);
        let w = fr.w / 3;
        for (i, lab) in ["X", "Y", "Z"].into_iter().enumerate() {
            let br = Rect::new(fr.x + i as i32 * w, fr.y, if i == 2 { fr.w - 2 * w } else { w - 1 }, ROW);
            if l.ui.flat_button(br, lab, v[i]) && !l.inactive {
                v[i] = !v[i];
                ch = true;
            }
        }
        l.y += ROW + GAP;
    }
    ch
}

/// An angle property shown in degrees, stored in radians.
fn degrees(l: &mut L, label: &str, key: &str, v: &mut f32, min: f32, max: f32) -> bool {
    let mut deg = v.to_degrees();
    if l.number(label, key, &mut deg, NumOpts { step: 1.0, unit: "°", min, max, ..PLAIN }) {
        *v = deg.to_radians();
        return true;
    }
    false
}

/// A dropdown of the scene's other mesh objects for a modifier's object field.
fn object_field(l: &mut L, d: &Doc, o: usize, si: usize, label: &str, cur: &str) {
    let mut items: Vec<&str> = vec![NO_OBJECT];
    items.extend(d.scene.objects.iter().enumerate().filter(|(i, x)| *i != o && matches!(x.data, ObjectData::Mesh(_))).map(|(_, x)| x.name.as_str()));
    l.enumeration(label, &format!("modobj/{si}"), cur, &items);
}

fn modifier_icon_at(l: &mut L, x: i32, y: i32) {
    icons::draw(l.ui.p, Icon::Modifier, x, y);
}

pub fn modifiers(l: &mut L, d: &mut Doc, c: &mut Ctx) {
    let Some(o) = c.obj else { return };
    if !matches!(d.scene.objects[o].data, ObjectData::Mesh(_)) {
        l.note("Modifiers need a mesh object");
        return;
    }
    l.space(2);
    if l.band(ROW) {
        let (x, w) = l.inner();
        let r = Rect::new(x, l.y, w, ROW);
        l.fields.push(("add_modifier".into(), r));
        if l.ui.dropdown(r, "     Add Modifier") {
            l.ui.open_menu(r, "", add_modifier_menu());
        }
        modifier_icon_at(l, x + 12, l.y + ROW / 2);
        l.y += ROW + 8;
    }
    let stack = d.settings.shown_stack(&d.scene, o);
    let oname = d.scene.objects[o].name.clone();
    for (si, (m, live, _)) in stack.into_iter().enumerate() {
        let key = format!("mod/{oname}/{si}/{}", m.name());
        let mut toggled = false;
        let mut menu_at = None;
        let mut remove = false;
        let title = label_of(&m);
        l.panel_with(
            d,
            &key,
            "",
            true,
            &mut |l, _, r| {
                // Icon, name, then the viewport toggle, menu and X at the right.
                icons::draw(l.ui.p, Icon::Modifier, r.x + 6, r.y + r.h / 2);
                let xr = Rect::new(r.right() - 20, r.y, 20, r.h);
                let mr = Rect::new(xr.x - 26, r.y, 24, r.h);
                let cam = Rect::new(mr.x - 26, r.y, 24, r.h);
                let vp = Rect::new(cam.x - 25, r.y, 24, r.h);
                let nr = Rect::new(r.x + 16, r.y, (vp.x - 4 - r.x - 16).max(10), r.h);
                l.ui.p.round_rect(nr, 4, t::BUTTON_DARK);
                l.ui.p.label(Rect::new(nr.x + 6, nr.y, nr.w - 8, nr.h), title, t::TEXT, Align::Left);
                for (rr, on) in [(vp, live), (cam, live)] {
                    if on {
                        l.ui.p.round_rect(rr, 4, t::SELECTED);
                    } else if l.ui.hovered(rr) {
                        l.ui.p.round_rect(rr, 4, t::BUTTON_HOVER);
                    }
                }
                icons::draw(l.ui.p, Icon::Monitor, vp.x + 12, vp.y + vp.h / 2);
                icons::draw(l.ui.p, if live { Icon::RenderOn } else { Icon::RenderOff }, cam.x + 12, cam.y + cam.h / 2);
                if l.ui.clicked(vp) || l.ui.clicked(cam) {
                    toggled = true;
                }
                if l.ui.hovered(mr) {
                    l.ui.p.round_rect(mr, 4, t::BUTTON_HOVER);
                }
                icons::draw(l.ui.p, Icon::Down, mr.x + 12, mr.y + mr.h / 2);
                if l.ui.clicked(mr) {
                    menu_at = Some(mr);
                }
                if l.ui.hovered(xr) {
                    l.ui.p.round_rect(xr, 4, t::BUTTON_HOVER);
                }
                icons::draw(l.ui.p, Icon::Cross, xr.x + 10, xr.y + xr.h / 2);
                if l.ui.clicked(xr) {
                    remove = true;
                }
            },
            &mut |l, d| {
                let was = l.inactive;
                l.inactive = was || !live;
                modifier_body(l, d, o, si, &m, live);
                l.inactive = was;
            },
        );
        if toggled {
            d.push_undo("Show in Viewport");
            d.settings.toggle_modifier(&mut d.scene, o, si);
            d.touch();
        }
        if remove {
            modifier_op(d, o, "object.modifier_remove", si);
        }
        if let Some(r) = menu_at {
            let op = |n: &str, k: &str, id: &str| MenuItem::op(n, k, Op::with(id, &si.to_string()));
            l.ui.open_menu(
                r,
                "",
                vec![
                    op("Apply", "Ctrl A", "object.modifier_apply"),
                    MenuItem::op("Apply as Shape Key", "", Op::with("properties.unavailable", "Apply as Shape Key")),
                    MenuItem::op("Copy to Selected", "", Op::with("properties.unavailable", "Copy to Selected")),
                    op("Duplicate", "Shift D", "object.modifier_copy"),
                    op("Delete", "X", "object.modifier_remove"),
                    MenuItem::Separator,
                    op("Move Up", "", "object.modifier_move_up"),
                    op("Move Down", "", "object.modifier_move_down"),
                    op("Move to First", "", "object.modifier_move_to_first"),
                    op("Move to Last", "", "object.modifier_move_to_last"),
                ],
            );
        }
    }
}

/// The modifier's real parameters, Blender's layout.
fn modifier_body(l: &mut L, d: &mut Doc, o: usize, si: usize, m: &Modifier, live: bool) {
    let mut nm = m.clone();
    let mut ch = false;
    let k = |s: &str| format!("m{si}/{s}");
    match &mut nm {
        Modifier::Subdivision { levels, simple } => {
            let cur = if *simple { 1 } else { 0 };
            if let Some(i) = l.expand("", &["Catmull-Clark", "Simple"], cur) {
                *simple = i == 1;
                ch = true;
            }
            ch |= l.uint("Levels Viewport", &k("levels"), levels, 0, 6);
            inactive(l, |l| {
                let mut r = *levels;
                l.uint("Render", &k("levels_render"), &mut r, 0, 6);
                let mut t = true;
                l.check("", "Optimal Display", &mut t);
            });
            l.panel(d, &format!("{}/adv", k("sub")), "Advanced", false, &mut |l, _| {
                inactive(l, |l| {
                    let mut t = true;
                    l.check("", "Use Limit Surface", &mut t);
                    l.enumeration("UV Smooth", "uvs", "Keep Boundaries", &[]);
                    l.enumeration("Boundary Smooth", "bs", "All", &[]);
                    l.check("", "Use Creases", &mut t);
                    let mut f = false;
                    l.check("", "Use Custom Normals", &mut f);
                });
            });
        }
        Modifier::Mirror { x, y, z, merge, merge_distance } => {
            if l.band(ROW) {
                let (lr, fr) = {
                    let (ix, iw) = l.inner();
                    let lw = (iw as f32 * 0.4) as i32;
                    (Rect::new(ix, l.y, lw - 8, ROW), Rect::new(ix + lw, l.y, iw - lw, ROW))
                };
                l.ui.p.label(lr, "Axis", t::TEXT, Align::Right);
                let w = fr.w / 3;
                for (i, (lab, v)) in [("X", &mut *x), ("Y", &mut *y), ("Z", &mut *z)].into_iter().enumerate() {
                    let br = Rect::new(fr.x + i as i32 * w, fr.y, if i == 2 { fr.w - 2 * w } else { w - 1 }, ROW);
                    if l.ui.flat_button(br, lab, *v) && !l.inactive {
                        *v = !*v;
                        ch = true;
                    }
                }
                l.y += ROW + GAP;
            }
            inactive(l, |l| {
                l.expand("Bisect", &["X", "Y", "Z"], 9);
                l.expand("Flip", &["X", "Y", "Z"], 9);
                l.enumeration("Mirror Object", "mo", "", &[]);
            });
            ch |= l.check("", "Merge", merge);
            ch |= l.number("Distance", &k("merge_d"), merge_distance, NumOpts { step: 0.001, min: 0.0, ..M });
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Clipping", &mut f);
            });
        }
        Modifier::Array { count, relative, constant, merge } => {
            l.info("Fit Type", "Fixed Count");
            ch |= l.uint("Count", &k("count"), count, 1, 1000);
            l.panel(d, &format!("{}/rel", k("sub")), "Relative Offset", true, &mut |l, _| {
                for (i, lab) in ["Factor X", "Y", "Z"].iter().enumerate() {
                    ch |= l.number(lab, &k(&format!("rel{i}")), &mut relative[i], NumOpts { step: 0.05, ..PLAIN });
                }
            });
            l.panel(d, &format!("{}/const", k("sub")), "Constant Offset", false, &mut |l, _| {
                for (i, lab) in ["Distance X", "Y", "Z"].iter().enumerate() {
                    ch |= l.number(lab, &k(&format!("const{i}")), &mut constant[i], M);
                }
            });
            l.panel(d, &format!("{}/merge", k("sub")), "Merge", false, &mut |l, _| {
                ch |= l.check("", "Merge", merge);
            });
        }
        Modifier::Solidify { thickness, offset } => {
            l.info("Mode", "Simple");
            ch |= l.number("Thickness", &k("thick"), thickness, NumOpts { step: 0.01, ..M });
            ch |= l.slider("Offset", &k("offset"), offset, -1.0, 1.0);
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Even Thickness", &mut f);
                let mut tt = true;
                l.check("Rim", "Fill", &mut tt);
            });
        }
        Modifier::Bevel { width, segments, angle } => {
            inactive(l, |l| {
                l.expand("", &["Vertices", "Edges"], 1);
                l.info("Width Type", "Offset");
            });
            ch |= l.number("Amount", &k("width"), width, NumOpts { step: 0.01, min: 0.0, ..M });
            ch |= l.uint("Segments", &k("segments"), segments, 1, 1000);
            l.info("Limit Method", "Angle");
            let mut deg = angle.to_degrees();
            if l.number("Angle", &k("angle"), &mut deg, NumOpts { step: 1.0, unit: "°", min: 0.0, max: 180.0, ..PLAIN }) {
                *angle = deg.to_radians();
                ch = true;
            }
        }
        Modifier::Triangulate => {
            inactive(l, |l| {
                l.enumeration("Quad Method", "qm", "Shortest Diagonal", &[]);
                l.enumeration("N-gon Method", "nm", "Beauty", &[]);
                let mut v = 4;
                l.uint("Minimum Vertices", "minv", &mut v, 4, 100);
            });
        }
        Modifier::Weld { distance } => {
            l.info("Mode", "All");
            ch |= l.number("Distance", &k("dist"), distance, NumOpts { step: 0.001, min: 0.0, ..M });
        }
        Modifier::Displace { strength, .. } => {
            inactive(l, |l| {
                l.field_button("Texture", "New");
                l.info("Coordinates", "Local");
            });
            l.info("Direction", "Normal");
            ch |= l.number("Strength", &k("strength"), strength, NumOpts { step: 0.05, ..PLAIN });
        }
        Modifier::Smooth { factor, iterations, .. } => {
            inactive(l, |l| {
                l.expand("Axis", &["X", "Y", "Z"], 9);
            });
            ch |= l.number("Factor", &k("factor"), factor, NumOpts { step: 0.05, ..PLAIN });
            ch |= l.uint("Repeat", &k("repeat"), iterations, 0, 10000);
        }
        Modifier::Decimate { mode, ratio, iterations, angle, triangulate } => {
            let cur = match mode {
                DecimateMode::Collapse => 0,
                DecimateMode::Unsubdivide => 1,
                DecimateMode::Planar => 2,
            };
            if let Some(i) = l.expand("", &["Collapse", "Un-Subdivide", "Planar"], cur) {
                *mode = [DecimateMode::Collapse, DecimateMode::Unsubdivide, DecimateMode::Planar][i];
                ch = true;
            }
            match mode {
                DecimateMode::Collapse => {
                    ch |= l.slider("Ratio", &k("ratio"), ratio, 0.0, 1.0);
                    inactive(l, |l| {
                        let mut f = false;
                        l.check("", "Symmetry", &mut f);
                    });
                    ch |= l.check("", "Triangulate", triangulate);
                }
                DecimateMode::Unsubdivide => ch |= l.uint("Iterations", &k("iter"), iterations, 0, 16),
                DecimateMode::Planar => {
                    ch |= degrees(l, "Angle Limit", &k("angle"), angle, 0.0, 180.0);
                    inactive(l, |l| {
                        l.info("Delimit", "Normal");
                        let mut f = false;
                        l.check("", "All Boundaries", &mut f);
                    });
                }
            }
        }
        Modifier::Remesh { mode, voxel_size, octree_depth, scale, smooth_shade } => {
            let cur = match mode {
                RemeshMode::Blocks => 0,
                RemeshMode::Smooth => 1,
                RemeshMode::Voxel => 2,
            };
            if let Some(i) = l.expand("", &["Blocks", "Smooth", "Voxel"], cur) {
                *mode = [RemeshMode::Blocks, RemeshMode::Smooth, RemeshMode::Voxel][i];
                ch = true;
            }
            if *mode == RemeshMode::Voxel {
                ch |= l.number("Voxel Size", &k("voxel"), voxel_size, NumOpts { step: 0.01, min: 0.0001, ..M });
                inactive(l, |l| {
                    let mut a = 0.0;
                    l.number("Adaptivity", "adapt", &mut a, M);
                });
            } else {
                ch |= l.uint("Octree Depth", &k("depth"), octree_depth, 1, 12);
                ch |= l.slider("Scale", &k("scale"), scale, 0.0, 0.99);
                inactive(l, |l| {
                    let mut f = false;
                    l.check("", "Remove Disconnected", &mut f);
                });
            }
            ch |= l.check("", "Smooth Shading", smooth_shade);
        }
        Modifier::Screw { axis, angle, steps, screw_offset, iterations, merge, merge_distance, flip, smooth_shade } => {
            ch |= degrees(l, "Angle", &k("angle"), angle, -36000.0, 36000.0);
            ch |= l.number("Screw", &k("screw"), screw_offset, M);
            ch |= l.uint("Iterations", &k("iter"), iterations, 1, 10000);
            if let Some(i) = l.expand("Axis", &["X", "Y", "Z"], *axis as usize) {
                *axis = i as u8;
                ch = true;
            }
            inactive(l, |l| l.enumeration("Axis Object", "axis_ob", "", &[]));
            ch |= l.uint("Steps Viewport", &k("steps"), steps, 1, 10000);
            inactive(l, |l| {
                let mut r = *steps;
                l.uint("Render", "steps_render", &mut r, 1, 10000);
            });
            ch |= l.check("Merge", "", merge);
            ch |= l.number("Distance", &k("merge_d"), merge_distance, NumOpts { step: 0.001, min: 0.0, ..M });
            l.panel(d, &format!("{}/normals", k("sub")), "Normals", true, &mut |l, _| {
                ch |= l.check("", "Smooth Shading", smooth_shade);
                inactive(l, |l| {
                    let mut f = false;
                    l.check("", "Calculate Order", &mut f);
                });
                ch |= l.check("", "Flip", flip);
            });
        }
        Modifier::Wireframe { thickness, offset, even, relative, boundary, replace } => {
            ch |= l.number("Thickness", &k("thick"), thickness, NumOpts { step: 0.01, ..M });
            ch |= l.number("Offset", &k("offset"), offset, NumOpts { step: 0.05, min: -1.0, max: 1.0, ..PLAIN });
            ch |= l.check("", "Boundary", boundary);
            ch |= l.check("", "Replace Original", replace);
            ch |= l.check("Thickness", "Even", even);
            ch |= l.check("", "Relative", relative);
            inactive(l, |l| {
                let mut f = false;
                l.check("Crease Edges", "", &mut f);
                let mut m = 0;
                l.int("Material Offset", "mat_ofs", &mut m, -32768, 32767);
            });
        }
        Modifier::EdgeSplit { angle, use_angle } => {
            ch |= l.check("Edge Angle", "", use_angle);
            ch |= degrees(l, "Split Angle", &k("angle"), angle, 0.0, 180.0);
            inactive(l, |l| {
                // Meshes carry no sharp-edge flags yet.
                let mut t = false;
                l.check("", "Sharp Edges", &mut t);
            });
        }
        Modifier::Build { start, length, reverse, random, seed } => {
            ch |= l.number("Start", &k("start"), start, NumOpts { step: 1.0, ..PLAIN });
            ch |= l.number("Length", &k("length"), length, NumOpts { step: 1.0, min: 1.0, ..PLAIN });
            ch |= l.check("", "Reversed", reverse);
            l.panel(d, &format!("{}/random", k("sub")), "Randomize", false, &mut |l, _| {
                ch |= l.check("", "Randomize", random);
                ch |= l.uint("Seed", &k("seed"), seed, 0, 1 << 30);
            });
        }
        Modifier::Boolean { object, operation } => {
            let cur = match operation {
                BooleanOp::Intersect => 0,
                BooleanOp::Union => 1,
                BooleanOp::Difference => 2,
            };
            if let Some(i) = l.expand("", &["Intersect", "Union", "Difference"], cur) {
                *operation = [BooleanOp::Intersect, BooleanOp::Union, BooleanOp::Difference][i];
                ch = true;
            }
            inactive(l, |l| {
                l.expand("Operand Type", &["Object", "Collection"], 0);
            });
            let shown = if object.is_empty() { "" } else { object.as_str() };
            object_field(l, d, o, si, "Object", shown);
            if !object.is_empty() && d.scene.find_object(object).is_none() {
                l.note(&format!("No object named '{object}'"));
            }
            l.info("Solver", "BSP (exact in general position)");
        }
        Modifier::SimpleDeform { mode, axis, factor, limits, lock, .. } => {
            let modes = [SimpleDeformMode::Twist, SimpleDeformMode::Bend, SimpleDeformMode::Taper, SimpleDeformMode::Stretch];
            let cur = modes.iter().position(|x| x == mode).unwrap_or(0);
            if let Some(i) = l.expand("", &["Twist", "Bend", "Taper", "Stretch"], cur) {
                *mode = modes[i];
                ch = true;
            }
            inactive(l, |l| l.enumeration("Origin", "origin", "", &[]));
            if matches!(mode, SimpleDeformMode::Twist | SimpleDeformMode::Bend) {
                ch |= degrees(l, "Angle", &k("angle"), factor, -36000.0, 36000.0);
            } else {
                ch |= l.number("Factor", &k("factor"), factor, NumOpts { step: 0.05, ..PLAIN });
            }
            if let Some(i) = l.expand("Axis", &["X", "Y", "Z"], *axis as usize) {
                *axis = i as u8;
                ch = true;
            }
            let ax = *axis as usize;
            l.panel(d, &format!("{}/restrict", k("sub")), "Restrictions", false, &mut |l, _| {
                ch |= l.slider("Limits", &k("lim0"), &mut limits[0], 0.0, 1.0);
                ch |= l.slider("", &k("lim1"), &mut limits[1], 0.0, 1.0);
                for (a, lab) in ["Lock X", "Lock Y", "Lock Z"].iter().enumerate() {
                    if a != ax {
                        ch |= l.check("", lab, &mut lock[a]);
                    }
                }
            });
        }
        Modifier::Cast { kind, factor, radius, size, axes, size_from_radius, .. } => {
            let kinds = [CastType::Sphere, CastType::Cylinder, CastType::Cuboid];
            let cur = kinds.iter().position(|x| x == kind).unwrap_or(0);
            if let Some(i) = l.expand("Shape", &["Sphere", "Cylinder", "Cuboid"], cur) {
                *kind = kinds[i];
                ch = true;
            }
            ch |= toggles(l, "Axis", axes);
            ch |= l.number("Factor", &k("factor"), factor, NumOpts { step: 0.05, ..PLAIN });
            ch |= l.number("Radius", &k("radius"), radius, NumOpts { step: 0.05, min: 0.0, ..M });
            ch |= l.number("Size", &k("size"), size, NumOpts { step: 0.05, min: 0.0, ..M });
            ch |= l.check("", "Size from Radius", size_from_radius);
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Use Transform", &mut f);
                l.enumeration("Control Object", "cast_ob", "", &[]);
            });
        }
        Modifier::Wave { x, y, cyclic, normals, falloff, height, width, narrowness, speed, time_offset, lifetime, damping, start, .. } => {
            ch |= l.check("Motion", "X", x);
            ch |= l.check("", "Y", y);
            ch |= l.check("", "Cyclic", cyclic);
            ch |= l.check("", "Along Normals", normals);
            ch |= l.number("Falloff", &k("falloff"), falloff, NumOpts { step: 0.1, min: 0.0, ..PLAIN });
            ch |= l.number("Height", &k("height"), height, NumOpts { step: 0.05, ..PLAIN });
            ch |= l.number("Width", &k("width"), width, NumOpts { step: 0.05, min: 0.0, ..PLAIN });
            ch |= l.number("Narrowness", &k("narrow"), narrowness, NumOpts { step: 0.05, min: 0.0, ..PLAIN });
            l.panel(d, &format!("{}/start", k("sub")), "Start Position", false, &mut |l, _| {
                ch |= l.number("Position X", &k("sx"), &mut start[0], M);
                ch |= l.number("Y", &k("sy"), &mut start[1], M);
            });
            l.panel(d, &format!("{}/time", k("sub")), "Time", false, &mut |l, _| {
                ch |= l.number("Offset", &k("toffs"), time_offset, NumOpts { step: 1.0, ..PLAIN });
                ch |= l.number("Life", &k("life"), lifetime, NumOpts { step: 1.0, ..PLAIN });
                ch |= l.number("Damping", &k("damp"), damping, NumOpts { step: 1.0, ..PLAIN });
                ch |= l.number("Speed", &k("speed"), speed, NumOpts { step: 0.01, ..PLAIN });
            });
        }
        Modifier::Shrinkwrap { target, method, offset, axes, positive, negative, .. } => {
            let methods = [ShrinkwrapMethod::NearestSurfacePoint, ShrinkwrapMethod::Project, ShrinkwrapMethod::NearestVertex];
            let cur = methods.iter().position(|x| x == method).unwrap_or(0);
            if let Some(i) = l.expand("Wrap Method", &["Nearest Surface", "Project", "Nearest Vertex"], cur) {
                *method = methods[i];
                ch = true;
            }
            inactive(l, |l| l.info("Snap Mode", "On Surface"));
            let shown = if target.is_empty() { "" } else { target.as_str() };
            object_field(l, d, o, si, "Target", shown);
            if !target.is_empty() && d.scene.find_object(target).is_none() {
                l.note(&format!("No object named '{target}'"));
            }
            ch |= l.number("Offset", &k("offset"), offset, NumOpts { step: 0.01, ..M });
            if *method == ShrinkwrapMethod::Project {
                ch |= toggles(l, "Axis", axes);
                ch |= l.check("", "Negative", negative);
                ch |= l.check("", "Positive", positive);
            }
        }
        Modifier::LaplacianSmooth { iterations, factor, border, axes, preserve_volume, normalized, .. } => {
            ch |= l.uint("Repeat", &k("repeat"), iterations, 0, 200);
            ch |= l.number("Factor", &k("factor"), factor, NumOpts { step: 0.05, ..PLAIN });
            ch |= l.number("Border", &k("border"), border, NumOpts { step: 0.05, ..PLAIN });
            ch |= toggles(l, "Axis", axes);
            ch |= l.check("", "Preserve Volume", preserve_volume);
            ch |= l.check("", "Normalized", normalized);
        }
        Modifier::CorrectiveSmooth { factor, iterations, scale, length_weighted, only_smooth, .. } => {
            ch |= l.number("Factor", &k("factor"), factor, NumOpts { step: 0.05, ..PLAIN });
            ch |= l.uint("Repeat", &k("repeat"), iterations, 0, 200);
            ch |= l.number("Scale", &k("scale"), scale, NumOpts { step: 0.05, ..PLAIN });
            if let Some(i) = l.expand("Smooth Type", &["Simple", "Length Weight"], *length_weighted as usize) {
                *length_weighted = i == 1;
                ch = true;
            }
            ch |= l.check("", "Only Smooth", only_smooth);
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Pin Boundaries", &mut f);
            });
            l.info("Rest Source", "Original Coordinates");
        }
        Modifier::WeightedNormal { keep_sharp } => {
            l.info("Weighting Mode", "Face Area");
            inactive(l, |l| {
                let mut w = 50;
                l.uint("Weight", "wn_weight", &mut w, 1, 100);
                let mut th = 0.01;
                l.number("Threshold", "wn_thresh", &mut th, PLAIN);
            });
            ch |= l.check("", "Keep Sharp", keep_sharp);
            inactive(l, |l| {
                let mut f = false;
                l.check("", "Face Influence", &mut f);
            });
            l.note("Smooth shading, area-weighted normals");
        }
        Modifier::Nodes { group, inputs } => {
            let mut items: Vec<&str> = vec![NO_OBJECT];
            items.extend(d.scene.node_groups.iter().filter(|g| g.kind == sex_3d::nodes::TreeKind::Geometry).map(|g| g.name.as_str()));
            l.enumeration("Node Group", &format!("modgroup/{si}"), if group.is_empty() { NO_OBJECT } else { group.as_str() }, &items);
            ch |= node_inputs(l, d, &k(""), group, inputs);
        }
        Modifier::Physics(PhysicsModifier::ParticleSystem(_)) => l.note("Settings are inside the Particles tab"),
        Modifier::Physics(_) => l.note("Settings are inside the Physics tab"),
        // Rigging modifiers have no panel yet.
        _ => {}
    }
    if ch && live && nm != *m {
        l.commit(d, "Modifier Property");
        // Entry `si` of the shown stack is live: find its live index.
        let shown = d.settings.shown_stack(&d.scene, o);
        if let Some((_, true, li)) = shown.get(si) {
            d.scene.objects[o].modifiers[*li] = nm;
        }
    }
}
