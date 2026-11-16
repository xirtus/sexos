// SPDX-License-Identifier: GPL-3.0-only
//! Drivers (Blender's `ChannelDriver`): a property's value computed from
//! variables read off other properties and objects, combined as an
//! Average, Sum, Minimum, Maximum or a scripted expression ([`super::expr`]).
//!
//! Variable types: Single Property (any data path on an object), Transform
//! Channel (an object's or bone's location, rotation or scale in world,
//! transform or local space), Distance and Rotational Difference between two
//! objects or bones.
use super::{expr, path};
use sex_3d::scene::Scene;
use sex_3d::{Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriverType {
    Average,
    Sum,
    Scripted,
    Min,
    Max,
}

impl DriverType {
    pub const ALL: [DriverType; 5] = [DriverType::Average, DriverType::Sum, DriverType::Scripted, DriverType::Min, DriverType::Max];
    pub fn id(self) -> &'static str {
        match self {
            DriverType::Average => "AVERAGE",
            DriverType::Sum => "SUM",
            DriverType::Scripted => "SCRIPTED",
            DriverType::Min => "MIN",
            DriverType::Max => "MAX",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            DriverType::Average => "Averaged Value",
            DriverType::Sum => "Sum Values",
            DriverType::Scripted => "Scripted Expression",
            DriverType::Min => "Minimum Value",
            DriverType::Max => "Maximum Value",
        }
    }
    pub fn from_arg(s: &str) -> Option<DriverType> {
        DriverType::ALL.into_iter().find(|d| d.id() == s || d.label() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarType {
    SingleProp,
    Transforms,
    LocDiff,
    RotDiff,
}

impl VarType {
    pub const ALL: [VarType; 4] = [VarType::SingleProp, VarType::Transforms, VarType::RotDiff, VarType::LocDiff];
    pub fn id(self) -> &'static str {
        match self {
            VarType::SingleProp => "SINGLE_PROP",
            VarType::Transforms => "TRANSFORMS",
            VarType::LocDiff => "LOC_DIFF",
            VarType::RotDiff => "ROTATION_DIFF",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            VarType::SingleProp => "Single Property",
            VarType::Transforms => "Transform Channel",
            VarType::LocDiff => "Distance",
            VarType::RotDiff => "Rotational Difference",
        }
    }
    pub fn from_arg(s: &str) -> Option<VarType> {
        VarType::ALL.into_iter().find(|d| d.id() == s || d.label() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformChannel {
    LocX,
    LocY,
    LocZ,
    RotX,
    RotY,
    RotZ,
    RotW,
    ScaleX,
    ScaleY,
    ScaleZ,
    ScaleAvg,
}

impl TransformChannel {
    pub const ALL: [TransformChannel; 11] = [
        TransformChannel::LocX,
        TransformChannel::LocY,
        TransformChannel::LocZ,
        TransformChannel::RotW,
        TransformChannel::RotX,
        TransformChannel::RotY,
        TransformChannel::RotZ,
        TransformChannel::ScaleX,
        TransformChannel::ScaleY,
        TransformChannel::ScaleZ,
        TransformChannel::ScaleAvg,
    ];
    pub fn id(self) -> &'static str {
        match self {
            TransformChannel::LocX => "LOC_X",
            TransformChannel::LocY => "LOC_Y",
            TransformChannel::LocZ => "LOC_Z",
            TransformChannel::RotX => "ROT_X",
            TransformChannel::RotY => "ROT_Y",
            TransformChannel::RotZ => "ROT_Z",
            TransformChannel::RotW => "ROT_W",
            TransformChannel::ScaleX => "SCALE_X",
            TransformChannel::ScaleY => "SCALE_Y",
            TransformChannel::ScaleZ => "SCALE_Z",
            TransformChannel::ScaleAvg => "SCALE_AVG",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TransformChannel::LocX => "X Location",
            TransformChannel::LocY => "Y Location",
            TransformChannel::LocZ => "Z Location",
            TransformChannel::RotX => "X Rotation",
            TransformChannel::RotY => "Y Rotation",
            TransformChannel::RotZ => "Z Rotation",
            TransformChannel::RotW => "W Rotation",
            TransformChannel::ScaleX => "X Scale",
            TransformChannel::ScaleY => "Y Scale",
            TransformChannel::ScaleZ => "Z Scale",
            TransformChannel::ScaleAvg => "Average Scale",
        }
    }
    pub fn from_arg(s: &str) -> Option<TransformChannel> {
        TransformChannel::ALL.into_iter().find(|d| d.id() == s || d.label() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformSpace {
    World,
    Transform,
    Local,
}

impl TransformSpace {
    pub const ALL: [TransformSpace; 3] = [TransformSpace::World, TransformSpace::Transform, TransformSpace::Local];
    pub fn id(self) -> &'static str {
        match self {
            TransformSpace::World => "WORLD_SPACE",
            TransformSpace::Transform => "TRANSFORM_SPACE",
            TransformSpace::Local => "LOCAL_SPACE",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            TransformSpace::World => "World Space",
            TransformSpace::Transform => "Transform Space",
            TransformSpace::Local => "Local Space",
        }
    }
    pub fn from_arg(s: &str) -> Option<TransformSpace> {
        TransformSpace::ALL.into_iter().find(|d| d.id() == s || d.label() == s)
    }
}

/// One target of a variable.
#[derive(Clone, Debug, PartialEq)]
pub struct DriverTarget {
    pub object: String,
    /// Single Property: the data path (with a trailing `[n]` for an element).
    pub path: String,
    /// Bone of an armature target (Transform/Distance/Rotational Difference).
    pub bone: String,
    pub channel: TransformChannel,
    pub space: TransformSpace,
}

impl Default for DriverTarget {
    fn default() -> Self {
        DriverTarget { object: String::new(), path: String::new(), bone: String::new(), channel: TransformChannel::LocX, space: TransformSpace::World }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DriverVar {
    pub name: String,
    pub kind: VarType,
    pub targets: [DriverTarget; 2],
    /// Last value, or the error that kept it from being read.
    pub value: f32,
    pub error: Option<String>,
}

impl DriverVar {
    pub fn new(name: &str, kind: VarType) -> DriverVar {
        DriverVar { name: name.into(), kind, targets: [DriverTarget::default(), DriverTarget::default()], value: 0.0, error: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Driver {
    /// The driven property.
    pub path: String,
    pub index: usize,
    pub kind: DriverType,
    pub expression: String,
    pub vars: Vec<DriverVar>,
    /// Optional mapping curve (the driver F-curve's keys); empty means the
    /// driver's value is used as it is.
    pub curve: Option<super::curve::FCurve>,
    pub mute: bool,
    pub select: bool,
    /// Last evaluated value and error.
    pub value: f32,
    pub error: Option<String>,
}

impl Driver {
    /// Blender's Ctrl+D default: a scripted expression `var` with one Single
    /// Property variable.
    pub fn new(path: &str, index: usize) -> Driver {
        Driver {
            path: path.into(),
            index,
            kind: DriverType::Scripted,
            expression: "var".into(),
            vars: vec![DriverVar::new("var", VarType::SingleProp)],
            curve: None,
            mute: false,
            select: false,
            value: 0.0,
            error: None,
        }
    }
    pub fn label(&self) -> String {
        let fc = super::curve::FCurve::new(&self.path, self.index);
        fc.label()
    }
}

/// World matrix of an object or of one of its bones (`pose_mat` in world).
fn target_world(scene: &Scene, t: &DriverTarget) -> Option<Mat4> {
    let oi = scene.find_object(&t.object)?;
    let w = scene.world_matrix(oi);
    if t.bone.is_empty() {
        return Some(w);
    }
    let arm = scene.armature_of(oi)?;
    let b = arm.find(&t.bone)?;
    let poses = scene.pose_matrices(oi);
    Some(w * *poses.get(b)?)
}

fn decompose(m: &Mat4) -> (Vec3, Quat, Vec3) {
    m.to_trs()
}

fn transform_value(scene: &Scene, t: &DriverTarget) -> Result<f32, String> {
    let oi = scene.find_object(&t.object).ok_or_else(|| format!("no object '{}'", t.object))?;
    let (loc, rot, scale) = match t.space {
        TransformSpace::World => decompose(&target_world(scene, t).ok_or("invalid target")?),
        TransformSpace::Transform | TransformSpace::Local => {
            if t.bone.is_empty() {
                let tr = scene.objects[oi].transform;
                (tr.location, tr.rotation, tr.scale)
            } else {
                let o = &scene.objects[oi];
                let pb = o.pose.iter().find(|p| p.name == t.bone);
                match pb {
                    Some(p) => {
                        let (l, r, s) = p.channel_matrix(false).to_trs();
                        (l, r, s)
                    }
                    None => (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE),
                }
            }
        }
    };
    let e = sex_3d::script::quat_to_euler_xyz(rot);
    Ok(match t.channel {
        TransformChannel::LocX => loc.x,
        TransformChannel::LocY => loc.y,
        TransformChannel::LocZ => loc.z,
        TransformChannel::RotX => e[0],
        TransformChannel::RotY => e[1],
        TransformChannel::RotZ => e[2],
        TransformChannel::RotW => rot.normalize().w,
        TransformChannel::ScaleX => scale.x,
        TransformChannel::ScaleY => scale.y,
        TransformChannel::ScaleZ => scale.z,
        TransformChannel::ScaleAvg => (scale.x + scale.y + scale.z) / 3.0,
    })
}

/// The value of one variable.
pub fn var_value(scene: &Scene, owner: usize, v: &DriverVar) -> Result<f32, String> {
    match v.kind {
        VarType::SingleProp => {
            let t = &v.targets[0];
            let oi = if t.object.is_empty() { Some(owner) } else { scene.find_object(&t.object) };
            let oi = oi.ok_or_else(|| format!("no object '{}'", t.object))?;
            if t.path.trim().is_empty() {
                return Err("no data path".into());
            }
            let (p, idx) = path::split_index(&t.path);
            path::get(scene, oi, p, idx.unwrap_or(0)).ok_or_else(|| format!("invalid path '{}'", t.path))
        }
        VarType::Transforms => transform_value(scene, &v.targets[0]),
        VarType::LocDiff => {
            let a = target_world(scene, &v.targets[0]).ok_or("invalid target 1")?;
            let b = target_world(scene, &v.targets[1]).ok_or("invalid target 2")?;
            Ok(a.translation_part().distance(b.translation_part()))
        }
        VarType::RotDiff => {
            let a = target_world(scene, &v.targets[0]).ok_or("invalid target 1")?.to_trs().1.normalize();
            let b = target_world(scene, &v.targets[1]).ok_or("invalid target 2")?.to_trs().1.normalize();
            let d = a.dot(b).abs().min(1.0);
            Ok(2.0 * d.acos())
        }
    }
}

/// Evaluate a driver at `frame` (variables are re-read), storing its value
/// and errors. Returns the value, or None when it failed.
pub fn evaluate(scene: &Scene, owner: usize, drv: &mut Driver, frame: f32) -> Option<f32> {
    let mut vals = Vec::with_capacity(drv.vars.len());
    let mut err = None;
    for v in &mut drv.vars {
        match var_value(scene, owner, v) {
            Ok(x) => {
                v.value = x;
                v.error = None;
                vals.push(x);
            }
            Err(e) => {
                v.error = Some(e.clone());
                err.get_or_insert(e);
                vals.push(0.0);
            }
        }
    }
    let result = match drv.kind {
        DriverType::Average => Ok(if vals.is_empty() { 0.0 } else { vals.iter().sum::<f32>() / vals.len() as f32 }),
        DriverType::Sum => Ok(vals.iter().sum()),
        DriverType::Min => Ok(vals.iter().copied().fold(f32::INFINITY, f32::min)).map(|v| if v.is_finite() { v } else { 0.0 }),
        DriverType::Max => Ok(vals.iter().copied().fold(f32::NEG_INFINITY, f32::max)).map(|v| if v.is_finite() { v } else { 0.0 }),
        DriverType::Scripted => {
            let names: Vec<(String, f64)> = drv.vars.iter().zip(&vals).map(|(v, &x)| (v.name.clone(), x as f64)).collect();
            expr::eval(&drv.expression, &|n| {
                if n == "frame" {
                    return Some(frame as f64);
                }
                names.iter().find(|(k, _)| k == n).map(|(_, v)| *v)
            })
            .map(|v| v as f32)
        }
    };
    match result {
        Ok(mut v) => {
            if let Some(c) = drv.curve.as_ref().filter(|c| !c.keys.is_empty()) {
                v = c.evaluate(v);
            }
            drv.value = v;
            drv.error = err;
            Some(v)
        }
        Err(e) => {
            drv.error = Some(e);
            None
        }
    }
}
