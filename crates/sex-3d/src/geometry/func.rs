// SPDX-License-Identifier: GPL-3.0-only
//! Per-element functions of pure nodes (Blender's multi-functions): the
//! shader math, colour and texture nodes run through the shader VM's
//! instructions, so geometry nodes compute exactly what Cycles does; the
//! function nodes of geometry trees (Compare, Boolean Math, Random Value,
//! rotation and matrix nodes, ...) are implemented here.
//!
//! A function takes the node's inputs in socket order (disabled sockets
//! included) and returns every output in socket order.
use super::attributes::Val;
use crate::math::{EulerOrder, Mat3, Mat4, Quat, Vec3};
use crate::nodes::{Node, Prop, SocketType, Value};
use crate::render::shader::color::{self, Blend};
use crate::render::shader::ops::{MapInterp, MappingType, MathOp, VecOp};
use crate::render::shader::tex::{self, NoiseType, VoronoiFeature, VoronoiMetric, WaveType};
use crate::render::shader::{vm::exec, Instr, Op, ShadeCtx, V4};
use std::sync::Arc;

pub type NodeFn = Arc<dyn Fn(&[Val]) -> Vec<Val> + Send + Sync>;

/// A socket's own value as a field element of the socket's type.
pub fn value_to_val(v: &Value, ty: SocketType) -> Val {
    let raw = match v {
        Value::Float(f) => Val::Float(*f),
        Value::Int(i) => Val::Int(*i),
        Value::Bool(b) => Val::Bool(*b),
        Value::Vector(x) => Val::Vector(Vec3::from_array(*x)),
        Value::Color(c) => Val::Color(*c),
        _ => Val::Float(0.0),
    };
    convert_to(raw, ty)
}

/// Convert to the element type of socket type `ty` (Blender's implicit
/// conversions between socket types).
pub fn convert_to(v: Val, ty: SocketType) -> Val {
    match ty {
        SocketType::Float => Val::Float(v.f()),
        SocketType::Int => Val::Int(v.i()),
        SocketType::Bool => Val::Bool(v.b()),
        SocketType::Vector => Val::Vector(v.v()),
        SocketType::Color => Val::Color(v.c()),
        SocketType::Rotation => Val::Rotation(v.q()),
        SocketType::Matrix => Val::Matrix(v.m()),
        _ => v,
    }
}

pub fn val_to_value(v: &Val) -> Value {
    match *v {
        Val::Float(f) => Value::Float(f),
        Val::Int(i) => Value::Int(i),
        Val::Bool(b) => Value::Bool(b),
        Val::Vector(x) => Value::Vector(x.to_array()),
        Val::Color(c) => Value::Color(c),
        Val::Rotation(q) => Value::Vector(q.to_euler(EulerOrder::XYZ)),
        Val::Matrix(_) => Value::None,
    }
}

#[derive(Clone, Copy)]
enum K {
    F,
    V,
    C,
}

fn to_v4(v: &Val, k: K) -> V4 {
    match k {
        K::F => {
            let f = v.f();
            [f, f, f, 1.0]
        }
        K::V => {
            let x = v.v();
            [x.x, x.y, x.z, 0.0]
        }
        K::C => v.c(),
    }
}

fn from_v4(r: V4, k: K) -> Val {
    match k {
        K::F => Val::Float(r[0]),
        K::V => Val::Vector(Vec3::new(r[0], r[1], r[2])),
        K::C => Val::Color(r),
    }
}

fn prop_i(n: &Node, k: &str, d: i32) -> i32 {
    match n.prop(k) {
        Some(Prop::Int(i)) => *i,
        Some(Prop::Float(f)) => *f as i32,
        _ => d,
    }
}

fn prop_f(n: &Node, k: &str, d: f32) -> f32 {
    match n.prop(k) {
        Some(Prop::Float(f)) => *f,
        Some(Prop::Int(i)) => *i as f32,
        _ => d,
    }
}

fn dims(s: &str) -> u8 {
    match s {
        "1D" => 1,
        "2D" => 2,
        "4D" => 4,
        _ => 3,
    }
}

/// A shader VM instruction for node `n`: (op, inputs by identifier with
/// kinds, outputs as (output socket index, kind)).
fn vm_op(n: &Node) -> Option<(Op, Vec<(&'static str, K)>, Vec<(usize, K)>)> {
    use K::{C, F, V};
    let out = |ids: &[(&str, K)]| -> Vec<(usize, K)> { ids.iter().map(|(id, k)| (n.outputs.iter().position(|s| s.identifier == *id).unwrap_or(usize::MAX), *k)).collect() };
    Some(match n.idname.as_str() {
        "ShaderNodeMath" => (Op::Math { op: MathOp::from_name(n.enum_prop("operation")), clamp: n.bool_prop("use_clamp") }, vec![("Value", F), ("Value_001", F), ("Value_002", F)], out(&[("Value", F)])),
        "ShaderNodeVectorMath" => (Op::VecMath(VecOp::from_name(n.enum_prop("operation"))), vec![("Vector", V), ("Vector_001", V), ("Vector_002", V), ("Scale", F)], out(&[("Vector", V), ("Value", F)])),
        "ShaderNodeMix" => {
            let clamp = !matches!(n.prop("clamp_factor"), Some(Prop::Bool(false)));
            match n.enum_prop("data_type") {
                "FLOAT" => (Op::MixFloat { clamp }, vec![("Factor_Float", F), ("A_Float", F), ("B_Float", F)], out(&[("Result_Float", F)])),
                "VECTOR" => {
                    let nu = n.enum_prop("factor_mode") == "NON_UNIFORM";
                    (Op::MixVec { clamp, non_uniform: nu }, vec![(if nu { "Factor_Vector" } else { "Factor_Float" }, if nu { V } else { F }), ("A_Vector", V), ("B_Vector", V)], out(&[("Result_Vector", V)]))
                }
                _ => (
                    Op::MixColor { blend: Blend::from_name(n.enum_prop("blend_type")), clamp_fac: clamp, clamp_result: n.bool_prop("clamp_result") },
                    vec![("Factor_Float", F), ("A_Color", C), ("B_Color", C)],
                    out(&[("Result_Color", C)]),
                ),
            }
        }
        "ShaderNodeMapRange" => {
            let vector = n.enum_prop("data_type") == "FLOAT_VECTOR";
            let op = Op::MapRange { interp: MapInterp::from_name(n.enum_prop("interpolation_type")), clamp: !matches!(n.prop("clamp"), Some(Prop::Bool(false))), vector };
            if vector {
                (op, vec![("Vector", V), ("From_Min_FLOAT3", V), ("From_Max_FLOAT3", V), ("To_Min_FLOAT3", V), ("To_Max_FLOAT3", V), ("Steps_FLOAT3", V)], out(&[("Result", F), ("Vector", V)]))
            } else {
                (op, vec![("Value", F), ("From Min", F), ("From Max", F), ("To Min", F), ("To Max", F), ("Steps", F)], out(&[("Result", F), ("Vector", V)]))
            }
        }
        "ShaderNodeClamp" => (Op::Clamp { range: n.enum_prop("clamp_type") == "RANGE" }, vec![("Value", F), ("Min", F), ("Max", F)], out(&[("Result", F)])),
        "ShaderNodeValToRGB" => {
            let Some(Prop::Ramp(r)) = n.prop("color_ramp") else { return None };
            (Op::Ramp(Arc::new(r.clone())), vec![("Fac", F)], out(&[("Color", C), ("Alpha", F)]))
        }
        "ShaderNodeFloatCurve" => {
            let Some(Prop::Curve(c)) = n.prop("mapping") else { return None };
            (Op::FloatCurve(Arc::new(c.clone())), vec![("Factor", F), ("Value", F)], out(&[("Value", F)]))
        }
        "ShaderNodeVectorCurve" => {
            let Some(Prop::Curve(c)) = n.prop("mapping") else { return None };
            (Op::VecCurve(Arc::new(c.clone())), vec![("Fac", F), ("Vector", V)], out(&[("Vector", V)]))
        }
        "ShaderNodeRGBCurve" => {
            let Some(Prop::Curve(c)) = n.prop("mapping") else { return None };
            (Op::RgbCurve(Arc::new(c.clone())), vec![("Fac", F), ("Color", C)], out(&[("Color", C)]))
        }
        "ShaderNodeCombineXYZ" => (Op::CombineXyz, vec![("X", F), ("Y", F), ("Z", F)], out(&[("Vector", V)])),
        "ShaderNodeSeparateXYZ" => (Op::SeparateXyz, vec![("Vector", V)], out(&[("X", F), ("Y", F), ("Z", F)])),
        "ShaderNodeVectorRotate" => {
            let ty = match n.enum_prop("rotation_type") {
                "X_AXIS" => 1,
                "Y_AXIS" => 2,
                "Z_AXIS" => 3,
                "EULER_XYZ" => 4,
                _ => 0,
            };
            (Op::VecRotate { ty, invert: n.bool_prop("invert") }, vec![("Vector", V), ("Center", V), ("Axis", V), ("Angle", F), ("Rotation", V)], out(&[("Vector", V)]))
        }
        "ShaderNodeMapping" => (Op::Mapping(MappingType::from_name(n.enum_prop("vector_type"))), vec![("Vector", V), ("Location", V), ("Rotation", V), ("Scale", V)], out(&[("Vector", V)])),
        "ShaderNodeBlackbody" => (Op::Blackbody, vec![("Temperature", F)], out(&[("Color", C)])),
        "ShaderNodeInvert" => (Op::Invert, vec![("Fac", F), ("Color", C)], out(&[("Color", C)])),
        "ShaderNodeGamma" => (Op::Gamma, vec![("Color", C), ("Gamma", F)], out(&[("Color", C)])),
        "ShaderNodeBrightContrast" => (Op::BrightContrast, vec![("Color", C), ("Bright", F), ("Contrast", F)], out(&[("Color", C)])),
        "ShaderNodeHueSaturation" => (Op::HueSat, vec![("Hue", F), ("Saturation", F), ("Value", F), ("Fac", F), ("Color", C)], out(&[("Color", C)])),
        "ShaderNodeRGBToBW" => (Op::RgbToBw, vec![("Color", C)], out(&[("Val", F)])),
        "ShaderNodeTexNoise" => (
            Op::Noise { dims: dims(n.enum_prop("noise_dimensions")), ty: NoiseType::from_name(n.enum_prop("noise_type")), normalize: !matches!(n.prop("normalize"), Some(Prop::Bool(false))) },
            vec![("Vector", V), ("W", F), ("Scale", F), ("Detail", F), ("Roughness", F), ("Lacunarity", F), ("Offset", F), ("Gain", F), ("Distortion", F)],
            out(&[("Fac", F), ("Color", C)]),
        ),
        "ShaderNodeTexVoronoi" => {
            let feature = match n.enum_prop("feature") {
                "F2" => VoronoiFeature::F2,
                "SMOOTH_F1" => VoronoiFeature::SmoothF1,
                "DISTANCE_TO_EDGE" => VoronoiFeature::DistanceToEdge,
                "N_SPHERE_RADIUS" => VoronoiFeature::NSphereRadius,
                _ => VoronoiFeature::F1,
            };
            let metric = match n.enum_prop("distance") {
                "MANHATTAN" => VoronoiMetric::Manhattan,
                "CHEBYCHEV" => VoronoiMetric::Chebychev,
                "MINKOWSKI" => VoronoiMetric::Minkowski,
                _ => VoronoiMetric::Euclidean,
            };
            (
                Op::Voronoi { dims: dims(n.enum_prop("voronoi_dimensions")), feature, metric, normalize: n.bool_prop("normalize") },
                vec![("Vector", V), ("W", F), ("Scale", F), ("Detail", F), ("Roughness", F), ("Lacunarity", F), ("Smoothness", F), ("Exponent", F), ("Randomness", F)],
                out(&[("Distance", F), ("Color", C), ("Position", V), ("W", F), ("Radius", F)]),
            )
        }
        "ShaderNodeTexWave" => {
            let rings = n.enum_prop("wave_type") == "RINGS";
            let direction = match n.enum_prop(if rings { "rings_direction" } else { "bands_direction" }) {
                "Y" => 1,
                "Z" => 2,
                "DIAGONAL" | "SPHERICAL" => 3,
                _ => 0,
            };
            let profile = match n.enum_prop("wave_profile") {
                "SAW" => 1,
                "TRI" => 2,
                _ => 0,
            };
            (
                Op::Wave { ty: if rings { WaveType::Rings } else { WaveType::Bands }, direction, profile },
                vec![("Vector", V), ("Scale", F), ("Distortion", F), ("Detail", F), ("Detail Scale", F), ("Detail Roughness", F), ("Phase Offset", F)],
                out(&[("Color", C), ("Fac", F)]),
            )
        }
        "ShaderNodeTexMagic" => (Op::Magic { depth: prop_i(n, "turbulence_depth", 2) }, vec![("Vector", V), ("Scale", F), ("Distortion", F)], out(&[("Color", C), ("Fac", F)])),
        "ShaderNodeTexBrick" => (
            Op::Brick { offset: prop_f(n, "offset", 0.5), offset_frequency: prop_i(n, "offset_frequency", 2), squash: prop_f(n, "squash", 1.0), squash_frequency: prop_i(n, "squash_frequency", 2) },
            vec![("Vector", V), ("Color1", C), ("Color2", C), ("Mortar", C), ("Scale", F), ("Mortar Size", F), ("Mortar Smooth", F), ("Bias", F), ("Brick Width", F), ("Row Height", F)],
            out(&[("Color", C), ("Fac", F)]),
        ),
        "ShaderNodeTexChecker" => (Op::Checker, vec![("Vector", V), ("Color1", C), ("Color2", C), ("Scale", F)], out(&[("Color", C), ("Fac", F)])),
        "ShaderNodeTexGradient" => {
            let ty = match n.enum_prop("gradient_type") {
                "QUADRATIC" => 1,
                "EASING" => 2,
                "DIAGONAL" => 3,
                "SPHERICAL" => 4,
                "QUADRATIC_SPHERE" => 5,
                "RADIAL" => 6,
                _ => 0,
            };
            (Op::Gradient(ty), vec![("Vector", V)], out(&[("Color", C), ("Fac", F)]))
        }
        "ShaderNodeTexWhiteNoise" => (Op::WhiteNoise { dims: dims(n.enum_prop("noise_dimensions")) }, vec![("Vector", V), ("W", F)], out(&[("Value", F), ("Color", C)])),
        "ShaderNodeTexGabor" => (
            Op::Gabor { three_d: n.enum_prop("gabor_type") == "3D" },
            vec![("Vector", V), ("Scale", F), ("Frequency", F), ("Anisotropy", F), ("Orientation 2D", F), ("Orientation 3D", V)],
            out(&[("Value", F), ("Phase", F), ("Intensity", F)]),
        ),
        _ => return None,
    })
}

fn vm_fn(n: &Node) -> Option<NodeFn> {
    let (op, ins, outs) = vm_op(n)?;
    let idx: Vec<(Option<usize>, K)> = ins.iter().map(|(id, k)| (n.inputs.iter().position(|s| s.identifier == *id), *k)).collect();
    let k = idx.len();
    let instr = Instr { op, ins: (0..k as u32).collect(), out: k as u32, variant: 0 };
    let defaults: Vec<Val> = n.outputs.iter().map(|s| value_to_val(&s.value, s.ty)).collect();
    let nout = n.outputs.len();
    Some(Arc::new(move |vals: &[Val]| {
        let mut regs: Vec<V4> = idx.iter().map(|(i, k)| i.and_then(|i| vals.get(i)).map_or([0.0; 4], |v| to_v4(v, *k))).collect();
        regs.resize(k + 16, [0.0; 4]);
        exec(&instr, &ShadeCtx::default(), &mut regs);
        let mut o = defaults.clone();
        o.resize(nout, Val::Float(0.0));
        for (j, (si, kind)) in outs.iter().enumerate() {
            if *si < o.len() {
                o[*si] = from_v4(regs[k + j], *kind);
            }
        }
        o
    }))
}

fn hash_f(a: u32, b: u32) -> f32 {
    tex::hash_uint2(a, b) as f32 / u32::MAX as f32
}

fn hash_f3(a: u32, b: u32, c: u32) -> f32 {
    tex::hash_uint3(a, b, c) as f32 / u32::MAX as f32
}

/// Blender's Random Value: hashes of (ID, Seed).
pub fn random_value(data_type: &str, id: i32, seed: i32, min: Val, max: Val, probability: f32) -> Val {
    let (id, seed) = (id as u32, seed as u32);
    match data_type {
        "FLOAT_VECTOR" => {
            let (a, b) = (min.v(), max.v());
            let r = Vec3::new(hash_f3(id, seed, 0), hash_f3(id, seed, 1), hash_f3(id, seed, 2));
            Val::Vector(a + (b - a).mul_elem(r))
        }
        "INT" => {
            let (a, b) = (min.i(), max.i());
            let t = hash_f(id, seed);
            let (lo, hi) = (a.min(b) as f64, a.max(b) as f64);
            Val::Int(((lo + (hi + 1.0 - lo) * t as f64).floor() as i64).clamp(lo as i64, hi as i64) as i32)
        }
        "BOOLEAN" => Val::Bool(hash_f(id, seed) <= probability),
        _ => {
            let (a, b) = (min.f(), max.f());
            Val::Float(a + (b - a) * hash_f(id, seed))
        }
    }
}

fn int_math(op: &str, a: i32, b: i32, c: i32) -> i32 {
    let gcd = |mut a: i64, mut b: i64| {
        a = a.abs();
        b = b.abs();
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    };
    match op {
        "SUBTRACT" => a.wrapping_sub(b),
        "MULTIPLY" => a.wrapping_mul(b),
        "DIVIDE" => {
            if b == 0 {
                0
            } else {
                a.wrapping_div(b)
            }
        }
        "MULTIPLY_ADD" => a.wrapping_mul(b).wrapping_add(c),
        "ABSOLUTE" => a.wrapping_abs(),
        "NEGATE" => a.wrapping_neg(),
        "POWER" => {
            if b < 0 {
                if a == 1 {
                    1
                } else {
                    0
                }
            } else {
                a.wrapping_pow(b as u32)
            }
        }
        "MINIMUM" => a.min(b),
        "MAXIMUM" => a.max(b),
        "SIGN" => a.signum(),
        "DIVIDE_ROUND" => {
            if b == 0 {
                0
            } else {
                (a as f64 / b as f64).round() as i32
            }
        }
        "DIVIDE_FLOOR" => {
            if b == 0 {
                0
            } else {
                (a as f64 / b as f64).floor() as i32
            }
        }
        "DIVIDE_CEIL" => {
            if b == 0 {
                0
            } else {
                (a as f64 / b as f64).ceil() as i32
            }
        }
        "FLOORED_MODULO" => {
            if b == 0 {
                0
            } else {
                a.wrapping_sub(((a as f64 / b as f64).floor() as i32).wrapping_mul(b))
            }
        }
        "MODULO" => {
            if b == 0 {
                0
            } else {
                a.wrapping_rem(b)
            }
        }
        "GCD" => gcd(a as i64, b as i64) as i32,
        "LCM" => {
            let g = gcd(a as i64, b as i64);
            if g == 0 {
                0
            } else {
                ((a as i64 * b as i64).abs() / g) as i32
            }
        }
        _ => a.wrapping_add(b),
    }
}

fn compare(n: &Node, v: &[Val], get: &dyn Fn(&str) -> Val) -> bool {
    let op = n.enum_prop("operation");
    let eps = get("Epsilon").f();
    let cmp_f = |a: f32, b: f32| match op {
        "LESS_THAN" => a < b,
        "LESS_EQUAL" => a <= b,
        "GREATER_THAN" => a > b,
        "GREATER_EQUAL" => a >= b,
        "EQUAL" => (a - b).abs() <= eps,
        "NOT_EQUAL" => (a - b).abs() > eps,
        _ => false,
    };
    let _ = v;
    match n.enum_prop("data_type") {
        "INT" => {
            let (a, b) = (get("A_INT").i(), get("B_INT").i());
            match op {
                "LESS_THAN" => a < b,
                "LESS_EQUAL" => a <= b,
                "GREATER_THAN" => a > b,
                "GREATER_EQUAL" => a >= b,
                "EQUAL" => a == b,
                "NOT_EQUAL" => a != b,
                _ => false,
            }
        }
        "VECTOR" => {
            let (a, b) = (get("A_VEC3").v(), get("B_VEC3").v());
            match n.enum_prop("mode") {
                "LENGTH" => cmp_f(a.length(), b.length()),
                "AVERAGE" => cmp_f((a.x + a.y + a.z) / 3.0, (b.x + b.y + b.z) / 3.0),
                "DOT_PRODUCT" => cmp_f(a.dot(b), get("C").f()),
                "DIRECTION" => cmp_f(a.normalize().dot(b.normalize()).clamp(-1.0, 1.0).acos(), get("Angle").f()),
                _ => match op {
                    "EQUAL" => (a - b).abs().max_elem() <= eps,
                    "NOT_EQUAL" => (a - b).abs().max_elem() > eps,
                    _ => cmp_f(a.x, b.x) && cmp_f(a.y, b.y) && cmp_f(a.z, b.z),
                },
            }
        }
        "RGBA" => {
            let (a, b) = (get("A_COL").c(), get("B_COL").c());
            let l = |c: [f32; 4]| color::luminance([c[0], c[1], c[2]]);
            match op {
                "EQUAL" => (0..4).all(|i| (a[i] - b[i]).abs() <= eps),
                "NOT_EQUAL" => (0..4).any(|i| (a[i] - b[i]).abs() > eps),
                "BRIGHTER" => l(a) > l(b),
                "DARKER" => l(a) < l(b),
                _ => false,
            }
        }
        _ => cmp_f(get("A").f(), get("B").f()),
    }
}

fn axis_vec(s: &str) -> Vec3 {
    match s {
        "X" => Vec3::X,
        "Y" => Vec3::Y,
        _ => Vec3::Z,
    }
}

/// Blender's Align Rotation to Vector / Align Euler to Vector.
pub fn align_to_vector(rot: Quat, factor: f32, vector: Vec3, axis: &str, pivot: &str) -> Quat {
    if vector.length_squared() < 1e-12 {
        return rot;
    }
    let local_axis = axis_vec(axis);
    let cur = rot.rotate(local_axis).normalize();
    let target = vector.normalize();
    let q = if pivot == "AUTO" {
        Quat::from_rotation_arc(cur, target)
    } else {
        let piv = rot.rotate(axis_vec(pivot)).normalize();
        let a = (cur - piv * cur.dot(piv)).normalize();
        let b = (target - piv * target.dot(piv)).normalize();
        if a.length_squared() < 0.5 || b.length_squared() < 0.5 {
            return rot;
        }
        let ang = a.cross(b).dot(piv).atan2(a.dot(b));
        Quat::from_axis_angle(piv, ang)
    };
    let q = Quat::IDENTITY.slerp(q, factor);
    (q * rot).normalize()
}

fn hsv_or_hsl(mode: &str, c: [f32; 3], to_rgb: bool) -> [f32; 3] {
    match (mode, to_rgb) {
        ("HSV", true) => color::hsv_to_rgb(c),
        ("HSL", true) => color::hsl_to_rgb(c),
        ("HSV", false) => color::rgb_to_hsv(c),
        ("HSL", false) => color::rgb_to_hsl(c),
        _ => c,
    }
}

/// The per-element function of a pure node, or None when the node is not
/// a pure function.
pub fn node_fn(n: &Node) -> Option<NodeFn> {
    if let Some(f) = vm_fn(n) {
        return Some(f);
    }
    let node = n.clone();
    let idx = |id: &str| node.input(id);
    let input_ids: Vec<String> = n.inputs.iter().map(|s| s.identifier.clone()).collect();
    let getter = move |vals: &[Val], id: &str| -> Val { input_ids.iter().position(|x| x == id).and_then(|i| vals.get(i).copied()).unwrap_or(Val::Float(0.0)) };
    let _ = idx;
    let nout = n.outputs.len();
    let f: NodeFn = match n.idname.as_str() {
        "FunctionNodeBooleanMath" => {
            let op = n.enum_prop("operation").to_string();
            Arc::new(move |v| {
                let (a, b) = (getter(v, "Boolean").b(), getter(v, "Boolean_001").b());
                vec![Val::Bool(match op.as_str() {
                    "OR" => a || b,
                    "NOT" => !a,
                    "NAND" => !(a && b),
                    "NOR" => !(a || b),
                    "XNOR" => a == b,
                    "XOR" => a != b,
                    "IMPLY" => !a || b,
                    "NIMPLY" => a && !b,
                    _ => a && b,
                })]
            })
        }
        "FunctionNodeIntegerMath" => {
            let op = n.enum_prop("operation").to_string();
            Arc::new(move |v| vec![Val::Int(int_math(&op, getter(v, "Value").i(), getter(v, "Value_001").i(), getter(v, "Value_002").i()))])
        }
        "FunctionNodeCompare" => Arc::new(move |v| vec![Val::Bool(compare(&node, v, &|id| getter(v, id)))]),
        "FunctionNodeFloatToInt" => {
            let mode = n.enum_prop("rounding_mode").to_string();
            Arc::new(move |v| {
                let f = getter(v, "Float").f();
                vec![Val::Int(match mode.as_str() {
                    "FLOOR" => f.floor(),
                    "CEILING" => f.ceil(),
                    "TRUNCATE" => f.trunc(),
                    _ => f.round(),
                } as i32)]
            })
        }
        "FunctionNodeRandomValue" => {
            let dt = n.enum_prop("data_type").to_string();
            Arc::new(move |v| {
                let (mn, mx) = match dt.as_str() {
                    "FLOAT_VECTOR" => (getter(v, "Min"), getter(v, "Max")),
                    "INT" => (getter(v, "Min_002"), getter(v, "Max_002")),
                    _ => (getter(v, "Min_001"), getter(v, "Max_001")),
                };
                let r = random_value(&dt, getter(v, "ID").i(), getter(v, "Seed").i(), mn, mx, getter(v, "Probability").f());
                let mut o = vec![Val::Vector(Vec3::ZERO), Val::Float(0.0), Val::Int(0), Val::Bool(false)];
                let k = match dt.as_str() {
                    "FLOAT_VECTOR" => 0,
                    "INT" => 2,
                    "BOOLEAN" => 3,
                    _ => 1,
                };
                o[k] = r;
                o
            })
        }
        "FunctionNodeCombineColor" => {
            let mode = n.enum_prop("mode").to_string();
            Arc::new(move |v| {
                let c = hsv_or_hsl(&mode, [getter(v, "Red").f(), getter(v, "Green").f(), getter(v, "Blue").f()], true);
                vec![Val::Color([c[0], c[1], c[2], getter(v, "Alpha").f()])]
            })
        }
        "FunctionNodeSeparateColor" => {
            let mode = n.enum_prop("mode").to_string();
            Arc::new(move |v| {
                let c = getter(v, "Color").c();
                let s = hsv_or_hsl(&mode, [c[0], c[1], c[2]], false);
                vec![Val::Float(s[0]), Val::Float(s[1]), Val::Float(s[2]), Val::Float(c[3])]
            })
        }
        "FunctionNodeEulerToRotation" => Arc::new(move |v| vec![Val::Rotation(Quat::from_euler(getter(v, "Euler").v().to_array(), EulerOrder::XYZ))]),
        "FunctionNodeRotationToEuler" => Arc::new(move |v| vec![Val::Vector(Vec3::from_array(getter(v, "Rotation").q().to_euler(EulerOrder::XYZ)))]),
        "FunctionNodeAxisAngleToRotation" => Arc::new(move |v| {
            let axis = getter(v, "Axis").v();
            vec![Val::Rotation(if axis.length_squared() < 1e-12 { Quat::IDENTITY } else { Quat::from_axis_angle(axis, getter(v, "Angle").f()) })]
        }),
        "FunctionNodeRotationToAxisAngle" => Arc::new(move |v| {
            let (a, ang) = getter(v, "Rotation").q().to_axis_angle();
            vec![Val::Vector(a), Val::Float(ang)]
        }),
        "FunctionNodeQuaternionToRotation" => Arc::new(move |v| vec![Val::Rotation(Quat::new(getter(v, "W").f(), getter(v, "X").f(), getter(v, "Y").f(), getter(v, "Z").f()).normalize())]),
        "FunctionNodeRotationToQuaternion" => Arc::new(move |v| {
            let q = getter(v, "Rotation").q();
            vec![Val::Float(q.w), Val::Float(q.x), Val::Float(q.y), Val::Float(q.z)]
        }),
        "FunctionNodeRotateVector" => Arc::new(move |v| vec![Val::Vector(getter(v, "Rotation").q().rotate(getter(v, "Vector").v()))]),
        "FunctionNodeRotateRotation" => {
            let local = n.enum_prop("rotation_space") == "LOCAL";
            Arc::new(move |v| {
                let (r, by) = (getter(v, "Rotation").q(), getter(v, "Rotate By").q());
                vec![Val::Rotation((if local { r * by } else { by * r }).normalize())]
            })
        }
        "FunctionNodeInvertRotation" => Arc::new(move |v| vec![Val::Rotation(getter(v, "Rotation").q().conjugate())]),
        "FunctionNodeAlignRotationToVector" => {
            let (axis, pivot) = (n.enum_prop("axis").to_string(), n.enum_prop("pivot_axis").to_string());
            Arc::new(move |v| vec![Val::Rotation(align_to_vector(getter(v, "Rotation").q(), getter(v, "Factor").f(), getter(v, "Vector").v(), &axis, if pivot.is_empty() { "AUTO" } else { &pivot }))])
        }
        "FunctionNodeAlignEulerToVector" => {
            let (axis, pivot) = (n.enum_prop("axis").to_string(), n.enum_prop("pivot_axis").to_string());
            Arc::new(move |v| {
                let r = Quat::from_euler(getter(v, "Rotation").v().to_array(), EulerOrder::XYZ);
                let q = align_to_vector(r, getter(v, "Factor").f(), getter(v, "Vector").v(), &axis, if pivot.is_empty() { "AUTO" } else { &pivot });
                vec![Val::Vector(Vec3::from_array(q.to_euler(EulerOrder::XYZ)))]
            })
        }
        "FunctionNodeCombineTransform" => Arc::new(move |v| vec![Val::Matrix(Mat4::from_trs(getter(v, "Translation").v(), getter(v, "Rotation").q(), getter(v, "Scale").v()))]),
        "FunctionNodeSeparateTransform" => Arc::new(move |v| {
            let (t, r, s) = getter(v, "Transform").m().to_trs();
            vec![Val::Vector(t), Val::Rotation(r), Val::Vector(s)]
        }),
        "FunctionNodeTransformPoint" => Arc::new(move |v| vec![Val::Vector(getter(v, "Transform").m().transform_point(getter(v, "Vector").v()))]),
        "FunctionNodeTransformDirection" => Arc::new(move |v| vec![Val::Vector(getter(v, "Transform").m().transform_vector(getter(v, "Direction").v()))]),
        "FunctionNodeInvertMatrix" => Arc::new(move |v| {
            let m = getter(v, "Matrix").m();
            let d = m.determinant();
            vec![Val::Matrix(if d.abs() > 1e-12 { m.inverse() } else { Mat4::IDENTITY }), Val::Bool(d.abs() > 1e-12)]
        }),
        "FunctionNodeTransposeMatrix" => Arc::new(move |v| vec![Val::Matrix(getter(v, "Matrix").m().transpose())]),
        "FunctionNodeMatrixMultiply" => Arc::new(move |v| vec![Val::Matrix(getter(v, "Matrix").m() * getter(v, "Matrix_001").m())]),
        "FunctionNodeMatrixDeterminant" => Arc::new(move |v| vec![Val::Float(getter(v, "Matrix").m().determinant())]),
        "FunctionNodeRotationToMatrix" | "FunctionNodeEulerToMatrix" => Arc::new(move |v| vec![Val::Matrix(Mat3::from_quat(getter(v, "Rotation").q()).to_mat4())]),
        "GeometryNodeSwitch" => Arc::new(move |v| vec![if getter(v, "Switch").b() { getter(v, "True") } else { getter(v, "False") }]),
        "GeometryNodeIndexSwitch" => {
            let k = n.inputs.len().saturating_sub(1);
            Arc::new(move |v| {
                let i = v.first().map_or(0, |x| x.i());
                vec![if i >= 0 && (i as usize) < k { v[1 + i as usize] } else { v.get(1).map(|x| AttrTypeDefault::default_like(*x)).unwrap_or(Val::Float(0.0)) }]
            })
        }
        _ => return None,
    };
    let _ = nout;
    Some(f)
}

trait AttrTypeDefault {
    fn default_like(self) -> Val;
}

impl AttrTypeDefault for Val {
    fn default_like(self) -> Val {
        super::attributes::AttrType::of_val(&self).default_val()
    }
}
