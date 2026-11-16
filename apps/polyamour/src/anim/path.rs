// SPDX-License-Identifier: GPL-3.0-only
//! Blender data paths on an object: reading and writing the animatable
//! numbers an F-curve or a driver names (`location`, `rotation_euler`,
//! `pose.bones["Bone"].rotation_quaternion`,
//! `data.shape_keys.key_blocks["Key 1"].value`, `data.energy`, `data.lens`,
//! `modifiers["Subdivision"].levels`, `constraints["Copy Location"].influence`
//! ...), with Blender's array indices (W X Y Z for quaternions).
//!
//! Every path is relative to an object (Blender keys data-block properties on
//! the data-block; here they hang off the object with the `data.` prefix the
//! Python API would use from the object). Vector properties are read and
//! written whole ([`get_array`] / [`set_array`]) so a rotation stored as a
//! quaternion is decomposed once per write.
use sex_3d::armature::{PoseBone, RotationMode};
use sex_3d::modifiers::Modifier;
use sex_3d::scene::{LightKind, ObjectData, Scene};
use sex_3d::script::quat_to_euler_xyz;
use sex_3d::{Quat, Vec3};

/// One step of a data path.
#[derive(Clone, Debug, PartialEq)]
pub enum Seg {
    Name(String),
    Key(String),
    Index(usize),
}

/// Split a data path into names and subscripts: `pose.bones["B"].scale` is
/// `[Name(pose), Name(bones), Key(B), Name(scale)]`.
pub fn parse(path: &str) -> Vec<Seg> {
    let mut out = Vec::new();
    let b = path.as_bytes();
    let mut i = 0;
    let mut name = String::new();
    let flush = |name: &mut String, out: &mut Vec<Seg>| {
        if !name.is_empty() {
            out.push(Seg::Name(core::mem::take(name)));
        }
    };
    while i < b.len() {
        match b[i] {
            b'.' => {
                flush(&mut name, &mut out);
                i += 1;
            }
            b'[' => {
                flush(&mut name, &mut out);
                i += 1;
                if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                    let q = b[i];
                    i += 1;
                    let mut s = String::new();
                    while i < b.len() && b[i] != q {
                        if b[i] == b'\\' && i + 1 < b.len() {
                            i += 1;
                        }
                        s.push(b[i] as char);
                        i += 1;
                    }
                    // Closing quote and bracket.
                    while i < b.len() && b[i] != b']' {
                        i += 1;
                    }
                    i += 1;
                    out.push(Seg::Key(s));
                } else {
                    let start = i;
                    while i < b.len() && b[i] != b']' {
                        i += 1;
                    }
                    let n = path[start..i].trim().parse().unwrap_or(0);
                    i += 1;
                    out.push(Seg::Index(n));
                }
            }
            c => {
                name.push(c as char);
                i += 1;
            }
        }
    }
    flush(&mut name, &mut out);
    out
}

/// Quote a name for a data path (`"` and `\` escaped).
pub fn quote(name: &str) -> String {
    let mut s = String::from("\"");
    for c in name.chars() {
        if c == '"' || c == '\\' {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('"');
    s
}

pub fn bone_path(bone: &str, prop: &str) -> String {
    format!("pose.bones[{}].{prop}", quote(bone))
}
pub fn shape_key_path(key: &str) -> String {
    format!("data.shape_keys.key_blocks[{}].value", quote(key))
}

/// A path with a trailing `[n]` split off: (`location`, Some(1)) for
/// `location[1]` (drivers' Single Property targets name one element).
pub fn split_index(path: &str) -> (&str, Option<usize>) {
    let t = path.trim_end();
    if let Some(open) = t.rfind('[') {
        if t.ends_with(']') {
            if let Ok(n) = t[open + 1..t.len() - 1].trim().parse::<usize>() {
                return (&t[..open], Some(n));
            }
        }
    }
    (t, None)
}

/// The bone a path is about (`pose.bones["B"]...`).
pub fn bone_of(path: &str) -> Option<String> {
    match parse(path).as_slice() {
        [Seg::Name(p), Seg::Name(b), Seg::Key(name), ..] if p == "pose" && b == "bones" => Some(name.clone()),
        _ => None,
    }
}

/// The shape key a path is about.
pub fn shape_key_of(path: &str) -> Option<String> {
    match parse(path).as_slice() {
        [Seg::Name(d), Seg::Name(s), Seg::Name(k), Seg::Key(name), ..] if d == "data" && s == "shape_keys" && k == "key_blocks" => Some(name.clone()),
        _ => None,
    }
}

/// Blender's names for duplicate modifiers: "Subdivision", "Subdivision.001".
pub fn modifier_names(stack: &[Modifier]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(stack.len());
    for m in stack {
        let base = m.name();
        let n = out.iter().filter(|x| x.as_str() == base || x.strip_prefix(base).is_some_and(|r| r.starts_with('.'))).count();
        out.push(if n == 0 { base.to_string() } else { format!("{base}.{n:03}") });
    }
    out
}

fn pose_bone<'a>(pose: &'a [PoseBone], name: &str) -> Option<&'a PoseBone> {
    pose.iter().find(|p| p.name == name)
}

fn pose_bone_mut<'a>(pose: &'a mut Vec<PoseBone>, name: &str) -> &'a mut PoseBone {
    let i = match pose.iter().position(|p| p.name == name) {
        Some(i) => i,
        None => {
            pose.push(PoseBone::new(name));
            pose.len() - 1
        }
    };
    &mut pose[i]
}

fn quat_arr(q: Quat) -> Vec<f32> {
    vec![q.w, q.x, q.y, q.z]
}

fn arr_quat(a: &[f32]) -> Quat {
    Quat { w: a[0], x: a[1], y: a[2], z: a[3] }
}

/// The values of the property `path` on object `oi` (one element for
/// plain numbers), or None when the path does not resolve.
pub fn get_array(scene: &Scene, oi: usize, path: &str) -> Option<Vec<f32>> {
    let o = scene.objects.get(oi)?;
    let segs = parse(path);
    let n = |s: &Seg, w: &str| matches!(s, Seg::Name(x) if x == w);
    match segs.as_slice() {
        [Seg::Name(p)] => match p.as_str() {
            "location" => Some(o.transform.location.to_array().to_vec()),
            "rotation_euler" => Some(quat_to_euler_xyz(o.transform.rotation).to_vec()),
            "rotation_quaternion" => Some(quat_arr(o.transform.rotation)),
            "rotation_axis_angle" => {
                let (axis, a) = o.transform.rotation.normalize().to_axis_angle();
                Some(vec![a, axis.x, axis.y, axis.z])
            }
            "scale" => Some(o.transform.scale.to_array().to_vec()),
            "hide_viewport" => Some(vec![if o.visible { 0.0 } else { 1.0 }]),
            "pass_index" => Some(vec![o.pass_index as f32]),
            _ => None,
        },
        [p, b, Seg::Key(bone), Seg::Name(prop)] if n(p, "pose") && n(b, "bones") => {
            let rest = PoseBone::new(bone);
            let pb = pose_bone(&o.pose, bone).unwrap_or(&rest);
            Some(match prop.as_str() {
                "location" => pb.location.to_array().to_vec(),
                "rotation_quaternion" => quat_arr(pb.rotation_quaternion),
                "rotation_euler" => pb.rotation_euler.to_array().to_vec(),
                "rotation_axis_angle" => pb.rotation_axis_angle.to_vec(),
                "scale" => pb.scale.to_array().to_vec(),
                _ => return None,
            })
        }
        [p, b, Seg::Key(bone), c, Seg::Key(con), Seg::Name(prop)] if n(p, "pose") && n(b, "bones") && n(c, "constraints") && prop == "influence" => {
            let pb = pose_bone(&o.pose, bone)?;
            pb.constraints.iter().find(|c| c.name == *con).map(|c| vec![c.influence])
        }
        [c, Seg::Key(con), Seg::Name(prop)] if n(c, "constraints") && prop == "influence" => o.constraints.iter().find(|c| c.name == *con).map(|c| vec![c.influence]),
        [m, Seg::Key(name), Seg::Name(prop)] if n(m, "modifiers") => {
            let names = modifier_names(&o.modifiers);
            let i = names.iter().position(|x| x == name)?;
            modifier_get(&o.modifiers[i], prop)
        }
        [d, s, k, Seg::Key(key), Seg::Name(prop)] if n(d, "data") && n(s, "shape_keys") && n(k, "key_blocks") => {
            let ObjectData::Mesh(mi) = o.data else { return None };
            let sk = scene.meshes.get(mi)?.shape_keys.iter().find(|x| x.name == *key)?;
            Some(vec![match prop.as_str() {
                "value" => sk.value,
                "slider_min" => sk.slider_min,
                "slider_max" => sk.slider_max,
                _ => return None,
            }])
        }
        [d, s, Seg::Name(prop)] if n(d, "data") && n(s, "shape_keys") && prop == "eval_time" => {
            let ObjectData::Mesh(mi) = o.data else { return None };
            Some(vec![scene.meshes.get(mi)?.shape_key_settings.eval_time])
        }
        [d, Seg::Name(prop)] if n(d, "data") => match &o.data {
            ObjectData::Light(l) => Some(match prop.as_str() {
                "energy" => vec![l.power],
                "color" => l.color.to_array().to_vec(),
                "shadow_soft_size" => vec![l.radius],
                "spot_size" => match l.kind {
                    LightKind::Spot { angle, .. } => vec![angle],
                    _ => return None,
                },
                "spot_blend" => match l.kind {
                    LightKind::Spot { blend, .. } => vec![blend],
                    _ => return None,
                },
                "size" => match l.kind {
                    LightKind::Area { size } => vec![size],
                    _ => return None,
                },
                _ => return None,
            }),
            ObjectData::Camera(c) => Some(match prop.as_str() {
                "lens" => vec![c.focal_length(CAMERA_ASPECT)],
                "angle" => vec![c.fov_y],
                "clip_start" => vec![c.near],
                "clip_end" => vec![c.far],
                "shift_x" => vec![c.shift_x],
                "shift_y" => vec![c.shift_y],
                "ortho_scale" => vec![c.ortho_scale?],
                _ => return None,
            }),
            _ => None,
        },
        [d, f, Seg::Name(prop)] if n(d, "data") && n(f, "dof") => match &o.data {
            ObjectData::Camera(c) => Some(vec![match prop.as_str() {
                "focus_distance" => c.dof.focus_distance,
                "aperture_fstop" => c.dof.fstop,
                _ => return None,
            }]),
            _ => None,
        },
        [m, Seg::Name(prop)] if n(m, "active_material") => {
            let mat = scene.materials.get(*o.materials.first()?)?;
            Some(match prop.as_str() {
                "diffuse_color" => mat.base_color.to_vec(),
                "metallic" => vec![mat.metallic],
                "roughness" => vec![mat.roughness],
                _ => return None,
            })
        }
        _ => None,
    }
}

/// The aspect (height / width) `data.lens` converts the camera's field of
/// view with: Blender's default 1920 x 1080 output.
pub const CAMERA_ASPECT: f32 = 1080.0 / 1920.0;

fn modifier_get(m: &Modifier, prop: &str) -> Option<Vec<f32>> {
    let v = match (m, prop) {
        (Modifier::Subdivision { levels, .. }, "levels") => *levels as f32,
        (Modifier::Array { count, .. }, "count") => *count as f32,
        (Modifier::Array { relative, .. }, "relative_offset_displace") => return Some(relative.to_vec()),
        (Modifier::Solidify { thickness, .. }, "thickness") => *thickness,
        (Modifier::Bevel { width, .. }, "width") => *width,
        (Modifier::Bevel { segments, .. }, "segments") => *segments as f32,
        (Modifier::Displace { strength, .. }, "strength") => *strength,
        (Modifier::Smooth { factor, .. }, "factor") => *factor,
        (Modifier::Decimate { ratio, .. }, "ratio") => *ratio,
        (Modifier::SimpleDeform { factor, .. }, "factor" | "angle") => *factor,
        (Modifier::Cast { factor, .. }, "factor") => *factor,
        (Modifier::Wave { height, .. }, "height") => *height,
        (Modifier::Wave { speed, .. }, "speed") => *speed,
        (Modifier::Lattice { strength, .. }, "strength") => *strength,
        (Modifier::Hook { strength, .. }, "strength") => *strength,
        (Modifier::Warp { strength, .. }, "strength") => *strength,
        (Modifier::Screw { angle, .. }, "angle") => *angle,
        (Modifier::Wireframe { thickness, .. }, "thickness") => *thickness,
        (Modifier::Build { start, .. }, "frame_start") => *start,
        _ => return None,
    };
    Some(vec![v])
}

fn modifier_set(m: &mut Modifier, prop: &str, a: &[f32]) -> bool {
    let v = a[0];
    let u = |v: f32| v.round().max(0.0) as u32;
    match (m, prop) {
        (Modifier::Subdivision { levels, .. }, "levels") => *levels = u(v).min(6),
        (Modifier::Array { count, .. }, "count") => *count = u(v).clamp(1, 1000),
        (Modifier::Array { relative, .. }, "relative_offset_displace") if a.len() >= 3 => relative.copy_from_slice(&a[..3]),
        (Modifier::Solidify { thickness, .. }, "thickness") => *thickness = v,
        (Modifier::Bevel { width, .. }, "width") => *width = v.max(0.0),
        (Modifier::Bevel { segments, .. }, "segments") => *segments = u(v).max(1),
        (Modifier::Displace { strength, .. }, "strength") => *strength = v,
        (Modifier::Smooth { factor, .. }, "factor") => *factor = v,
        (Modifier::Decimate { ratio, .. }, "ratio") => *ratio = v.clamp(0.0, 1.0),
        (Modifier::SimpleDeform { factor, .. }, "factor" | "angle") => *factor = v,
        (Modifier::Cast { factor, .. }, "factor") => *factor = v,
        (Modifier::Wave { height, .. }, "height") => *height = v,
        (Modifier::Wave { speed, .. }, "speed") => *speed = v,
        (Modifier::Lattice { strength, .. }, "strength") => *strength = v,
        (Modifier::Hook { strength, .. }, "strength") => *strength = v,
        (Modifier::Warp { strength, .. }, "strength") => *strength = v,
        (Modifier::Screw { angle, .. }, "angle") => *angle = v,
        (Modifier::Wireframe { thickness, .. }, "thickness") => *thickness = v,
        (Modifier::Build { start, .. }, "frame_start") => *start = v,
        _ => return false,
    }
    true
}

/// Write the property `path` on object `oi`; `vals` holds every element
/// (use [`get_array`] first to change some). Returns false when the path
/// does not resolve.
pub fn set_array(scene: &mut Scene, oi: usize, path: &str, vals: &[f32]) -> bool {
    let Some(len) = get_array(scene, oi, path).map(|v| v.len()) else { return false };
    if vals.len() < len {
        return false;
    }
    let segs = parse(path);
    let n = |s: &Seg, w: &str| matches!(s, Seg::Name(x) if x == w);
    let v3 = |a: &[f32]| Vec3::new(a[0], a[1], a[2]);
    let data = scene.objects[oi].data.clone();
    match segs.as_slice() {
        [Seg::Name(p)] => {
            let o = &mut scene.objects[oi];
            match p.as_str() {
                "location" => o.transform.location = v3(vals),
                "rotation_euler" => o.transform.rotation = Quat::from_euler_xyz(vals[0], vals[1], vals[2]),
                "rotation_quaternion" => o.transform.rotation = arr_quat(vals).normalize(),
                "rotation_axis_angle" => o.transform.rotation = Quat::from_axis_angle(v3(&vals[1..]).normalize(), vals[0]),
                "scale" => o.transform.scale = v3(vals),
                "hide_viewport" => o.visible = vals[0] < 0.5,
                "pass_index" => o.pass_index = vals[0].round().max(0.0) as u32,
                _ => return false,
            }
        }
        [p, b, Seg::Key(bone), Seg::Name(prop)] if n(p, "pose") && n(b, "bones") => {
            let pb = pose_bone_mut(&mut scene.objects[oi].pose, bone);
            match prop.as_str() {
                "location" => pb.location = v3(vals),
                "rotation_quaternion" => pb.rotation_quaternion = arr_quat(vals),
                "rotation_euler" => pb.rotation_euler = v3(vals),
                "rotation_axis_angle" => pb.rotation_axis_angle = [vals[0], vals[1], vals[2], vals[3]],
                "scale" => pb.scale = v3(vals),
                _ => return false,
            }
        }
        [p, b, Seg::Key(bone), _, Seg::Key(con), _] if n(p, "pose") && n(b, "bones") => {
            let Some(pb) = scene.objects[oi].pose.iter_mut().find(|p| p.name == *bone) else { return false };
            let Some(c) = pb.constraints.iter_mut().find(|c| c.name == *con) else { return false };
            c.influence = vals[0].clamp(0.0, 1.0);
        }
        [_, Seg::Key(con), _] if n(&segs[0], "constraints") => {
            let Some(c) = scene.objects[oi].constraints.iter_mut().find(|c| c.name == *con) else { return false };
            c.influence = vals[0].clamp(0.0, 1.0);
        }
        [_, Seg::Key(name), Seg::Name(prop)] if n(&segs[0], "modifiers") => {
            let names = modifier_names(&scene.objects[oi].modifiers);
            let Some(i) = names.iter().position(|x| x == name) else { return false };
            if !modifier_set(&mut scene.objects[oi].modifiers[i], prop, vals) {
                return false;
            }
        }
        [_, _, _, Seg::Key(key), Seg::Name(prop)] => {
            let ObjectData::Mesh(mi) = data else { return false };
            let Some(sk) = scene.meshes[mi].shape_keys.iter_mut().find(|x| x.name == *key) else { return false };
            match prop.as_str() {
                "value" => sk.value = vals[0].clamp(sk.slider_min.min(sk.slider_max), sk.slider_max.max(sk.slider_min)),
                "slider_min" => sk.slider_min = vals[0],
                "slider_max" => sk.slider_max = vals[0],
                _ => return false,
            }
        }
        [d, s, _] if n(d, "data") && n(s, "shape_keys") => {
            let ObjectData::Mesh(mi) = data else { return false };
            scene.meshes[mi].shape_key_settings.eval_time = vals[0];
        }
        [d, Seg::Name(prop)] if n(d, "data") => match &mut scene.objects[oi].data {
            ObjectData::Light(l) => match prop.as_str() {
                "energy" => l.power = vals[0].max(0.0),
                "color" => l.color = v3(vals),
                "shadow_soft_size" => l.radius = vals[0].max(0.0),
                "spot_size" => {
                    if let LightKind::Spot { angle, .. } = &mut l.kind {
                        *angle = vals[0].clamp(1f32.to_radians(), core::f32::consts::PI);
                    }
                }
                "spot_blend" => {
                    if let LightKind::Spot { blend, .. } = &mut l.kind {
                        *blend = vals[0].clamp(0.0, 1.0);
                    }
                }
                "size" => {
                    if let LightKind::Area { size } = &mut l.kind {
                        *size = vals[0].max(0.0);
                    }
                }
                _ => return false,
            },
            ObjectData::Camera(c) => match prop.as_str() {
                "lens" => c.set_focal_length(vals[0].max(1.0), CAMERA_ASPECT),
                "angle" => c.fov_y = vals[0].clamp(1e-3, 3.1),
                "clip_start" => c.near = vals[0].max(1e-6),
                "clip_end" => c.far = vals[0],
                "shift_x" => c.shift_x = vals[0],
                "shift_y" => c.shift_y = vals[0],
                "ortho_scale" => c.ortho_scale = Some(vals[0].max(1e-4)),
                _ => return false,
            },
            _ => return false,
        },
        [_, f, Seg::Name(prop)] if n(f, "dof") => match &mut scene.objects[oi].data {
            ObjectData::Camera(c) => match prop.as_str() {
                "focus_distance" => c.dof.focus_distance = vals[0].max(0.0),
                "aperture_fstop" => c.dof.fstop = vals[0].max(0.1),
                _ => return false,
            },
            _ => return false,
        },
        [m, Seg::Name(prop)] if n(m, "active_material") => {
            let Some(&mi) = scene.objects[oi].materials.first() else { return false };
            let mat = &mut scene.materials[mi];
            match prop.as_str() {
                "diffuse_color" => mat.base_color.copy_from_slice(&vals[..4]),
                "metallic" => mat.metallic = vals[0].clamp(0.0, 1.0),
                "roughness" => mat.roughness = vals[0].clamp(0.0, 1.0),
                _ => return false,
            }
        }
        _ => return false,
    }
    true
}

pub fn get(scene: &Scene, oi: usize, path: &str, index: usize) -> Option<f32> {
    get_array(scene, oi, path)?.get(index).copied()
}

/// Write one element of a property.
pub fn set(scene: &mut Scene, oi: usize, path: &str, index: usize, v: f32) -> bool {
    let Some(mut a) = get_array(scene, oi, path) else { return false };
    let Some(x) = a.get_mut(index) else { return false };
    *x = v;
    set_array(scene, oi, path, &a)
}

/// The property's own name in Blender's UI (the last path element).
pub fn prop_label(prop: &str) -> &'static str {
    match prop {
        "location" => "Location",
        "rotation_euler" => "Euler Rotation",
        "rotation_quaternion" => "Quaternion Rotation",
        "rotation_axis_angle" => "Axis-Angle Rotation",
        "scale" => "Scale",
        "value" => "Value",
        "slider_min" => "Slider Min",
        "slider_max" => "Slider Max",
        "eval_time" => "Evaluation Time",
        "influence" => "Influence",
        "energy" => "Power",
        "color" => "Color",
        "shadow_soft_size" => "Radius",
        "spot_size" => "Spot Size",
        "spot_blend" => "Spot Blend",
        "size" => "Size",
        "lens" => "Focal Length",
        "angle" => "Field of View",
        "clip_start" => "Clip Start",
        "clip_end" => "Clip End",
        "shift_x" => "Shift X",
        "shift_y" => "Shift Y",
        "ortho_scale" => "Orthographic Scale",
        "focus_distance" => "Focus Distance",
        "aperture_fstop" => "F-Stop",
        "diffuse_color" => "Viewport Display Color",
        "metallic" => "Metallic",
        "roughness" => "Roughness",
        "levels" => "Levels Viewport",
        "count" => "Count",
        "thickness" => "Thickness",
        "width" => "Amount",
        "segments" => "Segments",
        "strength" => "Strength",
        "factor" => "Factor",
        "ratio" => "Ratio",
        "height" => "Height",
        "speed" => "Speed",
        "hide_viewport" => "Disable in Viewports",
        "pass_index" => "Pass Index",
        "frame_start" => "Start",
        "relative_offset_displace" => "Relative Offset",
        _ => "Property",
    }
}

/// Blender's channel name: "X Location", "W Quaternion Rotation (Bone)",
/// "Value (Key 1)", "Power (Light)".
pub fn channel_label(path: &str, index: usize, array_len: usize) -> String {
    let segs = parse(path);
    let prop = match segs.last() {
        Some(Seg::Name(p)) => p.as_str(),
        _ => "",
    };
    let quat = prop == "rotation_quaternion" || prop == "rotation_axis_angle";
    let axis = if quat {
        ["W", "X", "Y", "Z"].get(index).copied().unwrap_or("")
    } else if prop == "color" || prop == "diffuse_color" {
        ["R", "G", "B", "A"].get(index).copied().unwrap_or("")
    } else if array_len > 1 || matches!(prop, "location" | "rotation_euler" | "scale") {
        ["X", "Y", "Z", "W"].get(index).copied().unwrap_or("")
    } else {
        ""
    };
    let base = if axis.is_empty() { prop_label(prop).to_string() } else { format!("{axis} {}", prop_label(prop)) };
    let owner = match segs.as_slice() {
        [Seg::Name(_)] => None,
        [_, _, Seg::Key(b), Seg::Name(_)] if path.starts_with("pose.") => Some(b.clone()),
        [.., Seg::Key(k), Seg::Name(_)] => Some(k.clone()),
        [Seg::Name(d), ..] if d == "data" => Some("Data".into()),
        [Seg::Name(m), ..] if m == "active_material" => Some("Material".into()),
        _ => None,
    };
    match owner {
        Some(o) => format!("{base} ({o})"),
        None => base,
    }
}

/// The channel group Blender files a new curve under: "Object Transforms"
/// for object transforms, the bone's name for pose bones, "Key" for shape
/// keys, nothing otherwise.
pub fn default_group(path: &str) -> String {
    if let Some(b) = bone_of(path) {
        return b;
    }
    if shape_key_of(path).is_some() {
        return "Key".into();
    }
    match path {
        "location" | "rotation_euler" | "rotation_quaternion" | "rotation_axis_angle" | "scale" => "Object Transforms".into(),
        _ => String::new(),
    }
}

/// Paths whose values are angles (shown in degrees in the Graph Editor).
pub fn is_angle(path: &str) -> bool {
    path.ends_with("rotation_euler") || path.ends_with("spot_size") || path.ends_with(".angle") || path == "data.angle"
}

/// The rotation path of a pose bone in its rotation mode.
pub fn bone_rotation_prop(mode: RotationMode) -> &'static str {
    match mode {
        RotationMode::Quaternion => "rotation_quaternion",
        RotationMode::Euler(_) => "rotation_euler",
        RotationMode::AxisAngle => "rotation_axis_angle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_resolves_paths() {
        assert_eq!(parse("pose.bones[\"Arm.L\"].location"), vec![Seg::Name("pose".into()), Seg::Name("bones".into()), Seg::Key("Arm.L".into()), Seg::Name("location".into())]);
        assert_eq!(split_index("location[2]"), ("location", Some(2)));
        assert_eq!(bone_of(&bone_path("B", "scale")).as_deref(), Some("B"));
        let mut s = Scene::startup();
        assert!(set(&mut s, 0, "location", 1, 3.0));
        assert_eq!(get(&s, 0, "location", 1), Some(3.0));
        assert!(set(&mut s, 0, "rotation_euler", 2, 0.5));
        assert!((get(&s, 0, "rotation_euler", 2).unwrap() - 0.5).abs() < 1e-5);
        let light = s.find_object("Light").unwrap();
        assert!(set(&mut s, light, "data.energy", 0, 250.0));
        assert_eq!(get(&s, light, "data.energy", 0), Some(250.0));
        assert_eq!(channel_label("rotation_quaternion", 0, 4), "W Quaternion Rotation");
        assert_eq!(channel_label(&bone_path("B", "location"), 1, 3), "Y Location (B)");
        assert_eq!(channel_label(&shape_key_path("Smile"), 0, 1), "Value (Smile)");
        assert_eq!(get(&s, 0, "nonsense", 0), None);
    }
}
