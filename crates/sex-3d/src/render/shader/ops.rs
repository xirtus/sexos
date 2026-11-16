// SPDX-License-Identifier: GPL-3.0-only
//! The Math, Vector Math, Map Range, Clamp, Mapping and Vector Rotate nodes,
//! after Cycles' `kernel/svm/math_util.h`, `map_range.h`, `mapping_util.h` and
//! `vector_rotate.h` (the "safe" variants: no NaNs from division by zero).
use crate::math::Vec3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    MultiplyAdd,
    Power,
    Logarithm,
    Sqrt,
    InverseSqrt,
    Absolute,
    Exponent,
    Minimum,
    Maximum,
    LessThan,
    GreaterThan,
    Sign,
    Compare,
    SmoothMin,
    SmoothMax,
    Round,
    Floor,
    Ceil,
    Trunc,
    Fract,
    Modulo,
    FlooredModulo,
    Wrap,
    Snap,
    PingPong,
    Sine,
    Cosine,
    Tangent,
    Sinh,
    Cosh,
    Tanh,
    Arcsine,
    Arccosine,
    Arctangent,
    Arctan2,
    Radians,
    Degrees,
}

impl MathOp {
    pub fn from_name(s: &str) -> MathOp {
        use MathOp::*;
        match s {
            "SUBTRACT" => Subtract,
            "MULTIPLY" => Multiply,
            "DIVIDE" => Divide,
            "MULTIPLY_ADD" => MultiplyAdd,
            "POWER" => Power,
            "LOGARITHM" => Logarithm,
            "SQRT" => Sqrt,
            "INVERSE_SQRT" => InverseSqrt,
            "ABSOLUTE" => Absolute,
            "EXPONENT" => Exponent,
            "MINIMUM" => Minimum,
            "MAXIMUM" => Maximum,
            "LESS_THAN" => LessThan,
            "GREATER_THAN" => GreaterThan,
            "SIGN" => Sign,
            "COMPARE" => Compare,
            "SMOOTH_MIN" => SmoothMin,
            "SMOOTH_MAX" => SmoothMax,
            "ROUND" => Round,
            "FLOOR" => Floor,
            "CEIL" => Ceil,
            "TRUNC" => Trunc,
            "FRACT" => Fract,
            "MODULO" => Modulo,
            "FLOORED_MODULO" => FlooredModulo,
            "WRAP" => Wrap,
            "SNAP" => Snap,
            "PINGPONG" => PingPong,
            "SINE" => Sine,
            "COSINE" => Cosine,
            "TANGENT" => Tangent,
            "SINH" => Sinh,
            "COSH" => Cosh,
            "TANH" => Tanh,
            "ARCSINE" => Arcsine,
            "ARCCOSINE" => Arccosine,
            "ARCTANGENT" => Arctangent,
            "ARCTAN2" => Arctan2,
            "RADIANS" => Radians,
            "DEGREES" => Degrees,
            _ => Add,
        }
    }

    /// `NodeMathOperation` numbers stored in files (`bNode.custom1`).
    pub fn file_name(n: i64) -> &'static str {
        const NAMES: [&str; 41] = [
            "ADD", "SUBTRACT", "MULTIPLY", "DIVIDE", "SINE", "COSINE", "TANGENT", "ARCSINE", "ARCCOSINE", "ARCTANGENT", "POWER", "LOGARITHM", "MINIMUM", "MAXIMUM", "ROUND",
            "LESS_THAN", "GREATER_THAN", "MODULO", "ABSOLUTE", "ARCTAN2", "FLOOR", "CEIL", "FRACT", "SQRT", "INVERSE_SQRT", "SIGN", "EXPONENT", "RADIANS", "DEGREES", "SINH",
            "COSH", "TANH", "TRUNC", "SNAP", "WRAP", "COMPARE", "MULTIPLY_ADD", "PINGPONG", "SMOOTH_MIN", "SMOOTH_MAX", "FLOORED_MODULO",
        ];
        NAMES.get(n as usize).copied().unwrap_or("ADD")
    }
}

pub fn safe_divide(a: f32, b: f32) -> f32 {
    if b != 0.0 {
        a / b
    } else {
        0.0
    }
}

fn safe_modulo(a: f32, b: f32) -> f32 {
    if b != 0.0 {
        a % b
    } else {
        0.0
    }
}

fn safe_floored_modulo(a: f32, b: f32) -> f32 {
    if b != 0.0 {
        a - (a / b).floor() * b
    } else {
        0.0
    }
}

fn compatible_powf(x: f32, y: f32) -> f32 {
    if y == 0.0 {
        return 1.0;
    }
    if x < 0.0 {
        let p = (-x).powf(y);
        return if (-y) % 2.0 == 0.0 { p } else { -p };
    }
    if x == 0.0 {
        return 0.0;
    }
    x.powf(y)
}

fn safe_powf(a: f32, b: f32) -> f32 {
    if a < 0.0 && b != b.trunc() {
        return 0.0;
    }
    compatible_powf(a, b)
}

fn safe_logf(a: f32, b: f32) -> f32 {
    if a <= 0.0 || b <= 0.0 {
        return 0.0;
    }
    safe_divide(a.ln(), b.ln())
}

fn smoothminf(a: f32, b: f32, k: f32) -> f32 {
    if k != 0.0 {
        let h = (k - (a - b).abs()).max(0.0) / k;
        a.min(b) - h * h * h * k * (1.0 / 6.0)
    } else {
        a.min(b)
    }
}

pub fn wrapf(value: f32, max: f32, min: f32) -> f32 {
    let range = max - min;
    if range != 0.0 {
        value - range * ((value - min) / range).floor()
    } else {
        min
    }
}

fn pingpongf(a: f32, b: f32) -> f32 {
    if b != 0.0 {
        let x = (a - b) / (b * 2.0);
        ((x - x.floor()) * b * 2.0 - b).abs()
    } else {
        0.0
    }
}

fn compatible_signf(f: f32) -> f32 {
    if f == 0.0 {
        0.0
    } else if f > 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// `svm_math`.
pub fn math(op: MathOp, a: f32, b: f32, c: f32) -> f32 {
    use MathOp::*;
    match op {
        Add => a + b,
        Subtract => a - b,
        Multiply => a * b,
        Divide => safe_divide(a, b),
        MultiplyAdd => a * b + c,
        Power => safe_powf(a, b),
        Logarithm => safe_logf(a, b),
        Sqrt => {
            if a > 0.0 {
                a.sqrt()
            } else {
                0.0
            }
        }
        InverseSqrt => {
            if a > 0.0 {
                1.0 / a.sqrt()
            } else {
                0.0
            }
        }
        Absolute => a.abs(),
        Exponent => a.exp(),
        Minimum => a.min(b),
        Maximum => a.max(b),
        LessThan => (a < b) as i32 as f32,
        GreaterThan => (a > b) as i32 as f32,
        Sign => compatible_signf(a),
        Compare => ((a - b).abs() <= c.max(f32::EPSILON)) as i32 as f32,
        SmoothMin => smoothminf(a, b, c),
        SmoothMax => -smoothminf(-a, -b, c),
        Round => (a + 0.5).floor(),
        Floor => a.floor(),
        Ceil => a.ceil(),
        Trunc => a.trunc(),
        Fract => a - a.floor(),
        Modulo => safe_modulo(a, b),
        FlooredModulo => safe_floored_modulo(a, b),
        Wrap => wrapf(a, b, c),
        Snap => (safe_divide(a, b)).floor() * b,
        PingPong => pingpongf(a, b),
        Sine => a.sin(),
        Cosine => a.cos(),
        Tangent => a.tan(),
        Sinh => a.sinh(),
        Cosh => a.cosh(),
        Tanh => a.tanh(),
        Arcsine => a.clamp(-1.0, 1.0).asin(),
        Arccosine => a.clamp(-1.0, 1.0).acos(),
        Arctangent => a.atan(),
        Arctan2 => a.atan2(b),
        Radians => a.to_radians(),
        Degrees => a.to_degrees(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VecOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    MultiplyAdd,
    CrossProduct,
    Project,
    Reflect,
    Refract,
    Faceforward,
    DotProduct,
    Distance,
    Length,
    Scale,
    Normalize,
    Snap,
    Floor,
    Ceil,
    Modulo,
    Wrap,
    Fraction,
    Absolute,
    Power,
    Sign,
    Minimum,
    Maximum,
    Sine,
    Cosine,
    Tangent,
}

impl VecOp {
    pub fn from_name(s: &str) -> VecOp {
        use VecOp::*;
        match s {
            "SUBTRACT" => Subtract,
            "MULTIPLY" => Multiply,
            "DIVIDE" => Divide,
            "MULTIPLY_ADD" => MultiplyAdd,
            "CROSS_PRODUCT" => CrossProduct,
            "PROJECT" => Project,
            "REFLECT" => Reflect,
            "REFRACT" => Refract,
            "FACEFORWARD" => Faceforward,
            "DOT_PRODUCT" => DotProduct,
            "DISTANCE" => Distance,
            "LENGTH" => Length,
            "SCALE" => Scale,
            "NORMALIZE" => Normalize,
            "SNAP" => Snap,
            "FLOOR" => Floor,
            "CEIL" => Ceil,
            "MODULO" => Modulo,
            "WRAP" => Wrap,
            "FRACTION" => Fraction,
            "ABSOLUTE" => Absolute,
            "POWER" => Power,
            "SIGN" => Sign,
            "MINIMUM" => Minimum,
            "MAXIMUM" => Maximum,
            "SINE" => Sine,
            "COSINE" => Cosine,
            "TANGENT" => Tangent,
            _ => Add,
        }
    }

    /// `NodeVectorMathOperation` numbers stored in files.
    pub fn file_name(n: i64) -> &'static str {
        const NAMES: [&str; 29] = [
            "ADD", "SUBTRACT", "MULTIPLY", "DIVIDE", "CROSS_PRODUCT", "PROJECT", "REFLECT", "DOT_PRODUCT", "DISTANCE", "LENGTH", "SCALE", "NORMALIZE", "SNAP", "FLOOR", "CEIL",
            "MODULO", "FRACTION", "ABSOLUTE", "MINIMUM", "MAXIMUM", "WRAP", "SINE", "COSINE", "TANGENT", "REFRACT", "FACEFORWARD", "MULTIPLY_ADD", "POWER", "SIGN",
        ];
        NAMES.get(n as usize).copied().unwrap_or("ADD")
    }
}

fn map(a: Vec3, f: impl Fn(f32) -> f32) -> Vec3 {
    Vec3::new(f(a.x), f(a.y), f(a.z))
}

fn zip(a: Vec3, b: Vec3, f: impl Fn(f32, f32) -> f32) -> Vec3 {
    Vec3::new(f(a.x, b.x), f(a.y, b.y), f(a.z, b.z))
}

pub fn safe_normalize(v: Vec3) -> Vec3 {
    let l = v.length();
    if l > 0.0 {
        v / l
    } else {
        Vec3::ZERO
    }
}

/// `svm_vector_math`: (vector, value).
pub fn vector_math(op: VecOp, a: Vec3, b: Vec3, c: Vec3, s: f32) -> (Vec3, f32) {
    use VecOp::*;
    let v = match op {
        Add => a + b,
        Subtract => a - b,
        Multiply => a.mul_elem(b),
        Divide => zip(a, b, safe_divide),
        MultiplyAdd => a.mul_elem(b) + c,
        CrossProduct => a.cross(b),
        Project => {
            let l2 = b.dot(b);
            if l2 != 0.0 {
                b * (a.dot(b) / l2)
            } else {
                Vec3::ZERO
            }
        }
        Reflect => {
            let n = safe_normalize(b);
            a - n * (2.0 * a.dot(n))
        }
        Refract => {
            let n = safe_normalize(b);
            let d = n.dot(a);
            let k = 1.0 - s * s * (1.0 - d * d);
            if k < 0.0 {
                Vec3::ZERO
            } else {
                a * s - n * (s * d + k.sqrt())
            }
        }
        Faceforward => {
            if c.dot(b) < 0.0 {
                a
            } else {
                -a
            }
        }
        DotProduct => return (Vec3::ZERO, a.dot(b)),
        Distance => return (Vec3::ZERO, a.distance(b)),
        Length => return (Vec3::ZERO, a.length()),
        Scale => a * s,
        Normalize => safe_normalize(a),
        Snap => zip(a, b, |x, y| safe_divide(x, y).floor() * y),
        Floor => map(a, f32::floor),
        Ceil => map(a, f32::ceil),
        Modulo => zip(a, b, safe_modulo),
        Wrap => Vec3::new(wrapf(a.x, b.x, c.x), wrapf(a.y, b.y, c.y), wrapf(a.z, b.z, c.z)),
        Fraction => map(a, |x| x - x.floor()),
        Absolute => map(a, f32::abs),
        Power => zip(a, b, safe_powf),
        Sign => map(a, compatible_signf),
        Minimum => zip(a, b, f32::min),
        Maximum => zip(a, b, f32::max),
        Sine => map(a, f32::sin),
        Cosine => map(a, f32::cos),
        Tangent => map(a, f32::tan),
    };
    (v, 0.0)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn smootherstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Map Range interpolation types (`interpolation_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapInterp {
    Linear,
    Stepped,
    SmoothStep,
    SmootherStep,
}

impl MapInterp {
    pub fn from_name(s: &str) -> MapInterp {
        match s {
            "STEPPED" => MapInterp::Stepped,
            "SMOOTHSTEP" => MapInterp::SmoothStep,
            "SMOOTHERSTEP" => MapInterp::SmootherStep,
            _ => MapInterp::Linear,
        }
    }
}

/// `svm_node_map_range` for one float.
#[allow(clippy::too_many_arguments)]
pub fn map_range(interp: MapInterp, clamp: bool, value: f32, from_min: f32, from_max: f32, to_min: f32, to_max: f32, steps: f32) -> f32 {
    if from_max == from_min {
        return 0.0;
    }
    let factor = match interp {
        MapInterp::Linear => (value - from_min) / (from_max - from_min),
        MapInterp::Stepped => {
            let f = (value - from_min) / (from_max - from_min);
            if steps > 0.0 {
                (f * (steps + 1.0)).floor() / steps
            } else {
                0.0
            }
        }
        MapInterp::SmoothStep => {
            if from_min > from_max {
                1.0 - smoothstep(from_max, from_min, value)
            } else {
                smoothstep(from_min, from_max, value)
            }
        }
        MapInterp::SmootherStep => {
            if from_min > from_max {
                1.0 - smootherstep(from_max, from_min, value)
            } else {
                smootherstep(from_min, from_max, value)
            }
        }
    };
    let r = to_min + factor * (to_max - to_min);
    if clamp && matches!(interp, MapInterp::Linear | MapInterp::Stepped) {
        if to_max > to_min {
            r.clamp(to_min, to_max)
        } else {
            r.clamp(to_max, to_min)
        }
    } else {
        r
    }
}

/// Clamp node: MINMAX, or RANGE (min and max swapped when reversed).
pub fn clamp_node(range: bool, v: f32, min: f32, max: f32) -> f32 {
    if range && min > max {
        v.clamp(max, min)
    } else {
        v.max(min).min(max)
    }
}

/// Blender's XYZ Euler rotation matrix (rows), R = Rz * Ry * Rx.
pub fn euler_to_mat(r: Vec3) -> [Vec3; 3] {
    let (ci, cj, ch) = (r.x.cos(), r.y.cos(), r.z.cos());
    let (si, sj, sh) = (r.x.sin(), r.y.sin(), r.z.sin());
    let (cc, cs, sc, ss) = (ci * ch, ci * sh, si * ch, si * sh);
    // Columns of Blender's `eul_to_mat3`, transposed into rows.
    let c0 = Vec3::new(cj * ch, cj * sh, -sj);
    let c1 = Vec3::new(sj * sc - cs, sj * ss + cc, cj * si);
    let c2 = Vec3::new(sj * cc + ss, sj * cs - sc, cj * ci);
    [Vec3::new(c0.x, c1.x, c2.x), Vec3::new(c0.y, c1.y, c2.y), Vec3::new(c0.z, c1.z, c2.z)]
}

pub fn mat_mul(m: &[Vec3; 3], v: Vec3) -> Vec3 {
    Vec3::new(m[0].dot(v), m[1].dot(v), m[2].dot(v))
}

pub fn mat_mul_t(m: &[Vec3; 3], v: Vec3) -> Vec3 {
    m[0] * v.x + m[1] * v.y + m[2] * v.z
}

/// Mapping node vector types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MappingType {
    Point,
    Texture,
    Vector,
    Normal,
}

impl MappingType {
    pub fn from_name(s: &str) -> MappingType {
        match s {
            "TEXTURE" => MappingType::Texture,
            "VECTOR" => MappingType::Vector,
            "NORMAL" => MappingType::Normal,
            _ => MappingType::Point,
        }
    }
}

/// `svm_mapping`.
pub fn mapping(ty: MappingType, v: Vec3, loc: Vec3, rot: Vec3, scale: Vec3) -> Vec3 {
    let m = euler_to_mat(rot);
    match ty {
        MappingType::Point => mat_mul(&m, v.mul_elem(scale)) + loc,
        MappingType::Texture => zip(mat_mul_t(&m, v - loc), scale, safe_divide),
        MappingType::Vector => mat_mul(&m, v.mul_elem(scale)),
        MappingType::Normal => safe_normalize(mat_mul(&m, zip(v, scale, safe_divide))),
    }
}

/// Rodrigues rotation of `p` about the unit `axis` by `angle` (right-handed).
pub fn rotate_around_axis(p: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    p * c + axis.cross(p) * s + axis * (axis.dot(p) * (1.0 - c))
}

/// Vector Rotate node: type 0 axis-angle, 1..3 X/Y/Z, 4 Euler XYZ.
pub fn vector_rotate(ty: u8, invert: bool, v: Vec3, center: Vec3, axis: Vec3, angle: f32, rot: Vec3) -> Vec3 {
    let sign = if invert { -1.0 } else { 1.0 };
    match ty {
        4 => {
            let m = euler_to_mat(rot);
            let d = v - center;
            (if invert { mat_mul_t(&m, d) } else { mat_mul(&m, d) }) + center
        }
        _ => {
            let axis = match ty {
                1 => Vec3::X,
                2 => Vec3::Y,
                3 => Vec3::Z,
                _ => {
                    let l = axis.length();
                    if l == 0.0 {
                        return v;
                    }
                    axis / l
                }
            };
            rotate_around_axis(v - center, axis, angle * sign) + center
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::f32::consts::{FRAC_PI_2, PI};

    #[test]
    fn math_ops() {
        use MathOp::*;
        let m = |op, a, b| math(op, a, b, 0.0);
        assert_eq!(m(Add, 1.0, 2.0), 3.0);
        assert_eq!(m(Divide, 1.0, 0.0), 0.0);
        assert_eq!(m(Power, -2.0, 3.0), -8.0);
        assert_eq!(m(Power, -2.0, 2.0), 4.0);
        assert_eq!(m(Power, -2.0, 0.5), 0.0);
        assert!((m(Logarithm, 8.0, 2.0) - 3.0).abs() < 1e-6);
        assert_eq!(m(Logarithm, -1.0, 2.0), 0.0);
        assert_eq!(m(Sqrt, -4.0, 0.0), 0.0);
        assert_eq!(m(InverseSqrt, 4.0, 0.0), 0.5);
        assert_eq!(m(Sign, -3.0, 0.0), -1.0);
        assert_eq!(m(Round, 2.5, 0.0), 3.0);
        assert_eq!(m(Round, -2.5, 0.0), -2.0);
        assert_eq!(m(Trunc, -2.7, 0.0), -2.0);
        assert!((m(Fract, -0.25, 0.0) - 0.75).abs() < 1e-6);
        assert_eq!(m(Modulo, -5.0, 3.0), -2.0);
        assert_eq!(m(FlooredModulo, -5.0, 3.0), 1.0);
        assert_eq!(m(Snap, 7.3, 2.0), 6.0);
        assert!((m(PingPong, 3.5, 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(math(Wrap, 5.5, 2.0, 0.0), 1.5);
        assert_eq!(math(Compare, 1.0, 1.05, 0.1), 1.0);
        assert_eq!(math(Compare, 1.0, 1.5, 0.1), 0.0);
        assert_eq!(math(MultiplyAdd, 2.0, 3.0, 4.0), 10.0);
        // Smooth min of equal values dips by k/6.
        assert!((math(SmoothMin, 1.0, 1.0, 0.6) - 0.9).abs() < 1e-6);
        assert!((math(SmoothMax, 1.0, 1.0, 0.6) - 1.1).abs() < 1e-6);
        assert_eq!(m(LessThan, 1.0, 2.0), 1.0);
        assert_eq!(m(Arcsine, 2.0, 0.0), FRAC_PI_2);
        assert!((m(Degrees, PI, 0.0) - 180.0).abs() < 1e-4);
        for (i, name) in ["ADD", "SUBTRACT", "MULTIPLY", "DIVIDE", "SINE"].iter().enumerate() {
            assert_eq!(MathOp::file_name(i as i64), *name);
        }
        assert_eq!(MathOp::file_name(40), "FLOORED_MODULO");
    }

    #[test]
    fn vector_math_ops() {
        use VecOp::*;
        let (a, b) = (Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, 2.0));
        let vm = |op| vector_math(op, a, b, Vec3::ZERO, 2.0);
        assert_eq!(vm(Add).0, Vec3::new(1.0, 2.0, 5.0));
        assert_eq!(vm(Divide).0, Vec3::new(0.0, 0.0, 1.5));
        assert_eq!(vm(CrossProduct).0, Vec3::new(4.0, -2.0, 0.0));
        assert_eq!(vm(Project).0, Vec3::new(0.0, 0.0, 3.0));
        assert_eq!(vm(Reflect).0, Vec3::new(1.0, 2.0, -3.0));
        assert_eq!(vm(DotProduct).1, 6.0);
        assert_eq!(vm(Length).1, 14f32.sqrt());
        assert_eq!(vm(Scale).0, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(vm(Snap).0, Vec3::new(0.0, 0.0, 2.0));
        assert_eq!(vm(Minimum).0, Vec3::new(0.0, 0.0, 2.0));
        // Refraction straight through with IOR ratio 1.
        let (r, _) = vector_math(Refract, Vec3::new(0.0, 0.0, -1.0), Vec3::Z, Vec3::ZERO, 1.0);
        assert!(r.approx_eq(Vec3::new(0.0, 0.0, -1.0), 1e-6));
        assert_eq!(vector_math(Faceforward, a, Vec3::Z, -Vec3::Z, 0.0).0, a);
        assert_eq!(VecOp::file_name(28), "SIGN");
    }

    #[test]
    fn map_range_and_clamp() {
        assert_eq!(map_range(MapInterp::Linear, true, 0.5, 0.0, 1.0, 10.0, 20.0, 4.0), 15.0);
        assert_eq!(map_range(MapInterp::Linear, true, 2.0, 0.0, 1.0, 10.0, 20.0, 4.0), 20.0);
        assert_eq!(map_range(MapInterp::Linear, false, 2.0, 0.0, 1.0, 10.0, 20.0, 4.0), 30.0);
        assert_eq!(map_range(MapInterp::Stepped, true, 0.6, 0.0, 1.0, 0.0, 1.0, 4.0), 0.75);
        assert!((map_range(MapInterp::SmoothStep, true, 0.25, 0.0, 1.0, 0.0, 1.0, 4.0) - 0.15625).abs() < 1e-6);
        assert_eq!(map_range(MapInterp::Linear, true, 0.5, 1.0, 1.0, 0.0, 1.0, 4.0), 0.0);
        assert_eq!(clamp_node(false, 5.0, 0.0, 1.0), 1.0);
        assert_eq!(clamp_node(true, 0.5, 1.0, 0.0), 0.5);
    }

    #[test]
    fn mapping_and_rotation() {
        let v = Vec3::new(1.0, 0.0, 0.0);
        // 90 degrees about Z turns X into Y.
        let r = mapping(MappingType::Point, v, Vec3::ZERO, Vec3::new(0.0, 0.0, FRAC_PI_2), Vec3::ONE);
        assert!(r.approx_eq(Vec3::Y, 1e-6), "{r:?}");
        let t = mapping(MappingType::Texture, r, Vec3::ZERO, Vec3::new(0.0, 0.0, FRAC_PI_2), Vec3::ONE);
        assert!(t.approx_eq(v, 1e-6));
        let p = mapping(MappingType::Point, v, Vec3::new(0.0, 0.0, 1.0), Vec3::ZERO, Vec3::splat(2.0));
        assert_eq!(p, Vec3::new(2.0, 0.0, 1.0));
        // XYZ order: X first, then Y, then Z.
        let e = mapping(MappingType::Vector, Vec3::Y, Vec3::ZERO, Vec3::new(FRAC_PI_2, 0.0, FRAC_PI_2), Vec3::ONE);
        assert!(e.approx_eq(Vec3::Z, 1e-6), "{e:?}");
        let q = vector_rotate(0, false, v, Vec3::ZERO, Vec3::Z, FRAC_PI_2, Vec3::ZERO);
        assert!(q.approx_eq(Vec3::Y, 1e-6));
        let q = vector_rotate(3, true, v, Vec3::ZERO, Vec3::Z, FRAC_PI_2, Vec3::ZERO);
        assert!(q.approx_eq(-Vec3::Y, 1e-6));
        let q = vector_rotate(4, false, v, Vec3::ZERO, Vec3::Z, 0.0, Vec3::new(0.0, 0.0, FRAC_PI_2));
        assert!(q.approx_eq(Vec3::Y, 1e-6));
    }
}
