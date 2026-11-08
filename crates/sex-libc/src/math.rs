// SPDX-License-Identifier: GPL-3.0-only
//! `<math.h>` over the `libm` crate (musl's maths in Rust). `long double` is
//! `double` in the SexOS C headers, so the `l` forms share the `double` code.
use crate::{c_int, c_long};

macro_rules! one { ($($n:ident $f:ident $nl:ident),*) => { $(
    #[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn $n(x: f64) -> f64 { libm::$n(x) }
    #[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn $f(x: f32) -> f32 { libm::$f(x) }
    #[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn $nl(x: f64) -> f64 { libm::$n(x) }
)* } }
macro_rules! two { ($($n:ident $f:ident $nl:ident),*) => { $(
    #[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn $n(x: f64, y: f64) -> f64 { libm::$n(x, y) }
    #[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn $f(x: f32, y: f32) -> f32 { libm::$f(x, y) }
    #[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn $nl(x: f64, y: f64) -> f64 { libm::$n(x, y) }
)* } }

one!(acos acosf acosl, asin asinf asinl, atan atanf atanl, cos cosf cosl, cosh coshf coshl, sin sinf sinl, sinh sinhf sinhl, tan tanf tanl, tanh tanhf tanhl, exp expf expl, exp2 exp2f exp2l, expm1 expm1f expm1l, log logf logl, log10 log10f log10l, log2 log2f log2l, log1p log1pf log1pl, sqrt sqrtf sqrtl, cbrt cbrtf cbrtl, ceil ceilf ceill, floor floorf floorl, round roundf roundl, trunc truncf truncl, rint rintf rintl, fabs fabsf fabsl, erf erff erfl, erfc erfcf erfcl, tgamma tgammaf tgammal, lgamma lgammaf lgammal);
two!(atan2 atan2f atan2l, pow powf powl, fmod fmodf fmodl, hypot hypotf hypotl, copysign copysignf copysignl, fmin fminf fminl, fmax fmaxf fmaxl, fdim fdimf fdiml, remainder remainderf remainderl, nextafter nextafterf nextafterl);

#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn ldexp(x: f64, e: c_int) -> f64 { libm::ldexp(x, e) }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn ldexpf(x: f32, e: c_int) -> f32 { libm::ldexpf(x, e) }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn scalbn(x: f64, e: c_int) -> f64 { libm::scalbn(x, e) }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn scalbnf(x: f32, e: c_int) -> f32 { libm::scalbnf(x, e) }
#[cfg_attr(target_os = "none", no_mangle)] pub unsafe extern "C" fn frexp(x: f64, e: *mut c_int) -> f64 { let (m, k) = libm::frexp(x); unsafe { *e = k }; m }
#[cfg_attr(target_os = "none", no_mangle)] pub unsafe extern "C" fn frexpf(x: f32, e: *mut c_int) -> f32 { let (m, k) = libm::frexpf(x); unsafe { *e = k }; m }
#[cfg_attr(target_os = "none", no_mangle)] pub unsafe extern "C" fn modf(x: f64, i: *mut f64) -> f64 { let (f, n) = libm::modf(x); unsafe { *i = n }; f }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn lrint(x: f64) -> c_long { libm::rint(x) as c_long }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn lrintf(x: f32) -> c_long { libm::rintf(x) as c_long }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn llrint(x: f64) -> i64 { libm::rint(x) as i64 }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn llrintf(x: f32) -> i64 { libm::rintf(x) as i64 }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn lround(x: f64) -> c_long { libm::round(x) as c_long }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn llround(x: f64) -> i64 { libm::round(x) as i64 }

/// musl's classification numbers: FP_NAN 0, FP_INFINITE 1, FP_ZERO 2, FP_SUBNORMAL 3, FP_NORMAL 4.
fn classify(bits: u64, exp_bits: u32, mant_bits: u32) -> c_int {
    let exp = (bits >> mant_bits) & ((1 << exp_bits) - 1);
    let mant = bits & ((1u64 << mant_bits) - 1);
    match (exp, mant) {
        (0, 0) => 2,
        (0, _) => 3,
        (e, 0) if e == (1 << exp_bits) - 1 => 1,
        (e, _) if e == (1 << exp_bits) - 1 => 0,
        _ => 4,
    }
}
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn __fpclassify(x: f64) -> c_int { classify(x.to_bits(), 11, 52) }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn __fpclassifyl(x: f64) -> c_int { classify(x.to_bits(), 11, 52) }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn __fpclassifyf(x: f32) -> c_int { classify(x.to_bits() as u64, 8, 23) }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn __signbit(x: f64) -> c_int { x.is_sign_negative() as c_int }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn __signbitf(x: f32) -> c_int { x.is_sign_negative() as c_int }
#[cfg_attr(target_os = "none", no_mangle)] pub extern "C" fn __signbitl(x: f64) -> c_int { x.is_sign_negative() as c_int }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_and_rounding() {
        assert_eq!(__fpclassify(0.0), 2);
        assert_eq!(__fpclassify(f64::MIN_POSITIVE / 2.0), 3);
        assert_eq!(__fpclassify(1.0), 4);
        assert_eq!(__fpclassify(f64::INFINITY), 1);
        assert_eq!(__fpclassify(f64::NAN), 0);
        assert_eq!(__fpclassifyf(f32::NAN), 0);
        assert_eq!(lrint(2.5), 2, "round half to even");
        assert_eq!(llrint(-3.5), -4);
        assert!((pow(2.0, 10.0) - 1024.0).abs() < 1e-9);
        let mut e = 0;
        assert_eq!(unsafe { frexp(8.0, &mut e) }, 0.5);
        assert_eq!(e, 4);
    }
}

