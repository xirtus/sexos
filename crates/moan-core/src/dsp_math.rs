// SPDX-License-Identifier: GPL-3.0-only
//! Accurate `#![no_std]` single-precision floating point math primitives for audio DSP.
//! Zero dependencies, deterministic cross-platform behavior.

pub const PI: f32 = 3.14159265358979323846;
pub const TAU: f32 = 6.28318530717958647692;
pub const FRAC_PI_2: f32 = 1.57079632679489661923;
pub const LN_2: f32 = 0.6931471805599453;
pub const LOG2_E: f32 = 1.4426950408889634;
pub const LOG10_2: f32 = 0.3010299956639812;

#[inline]
pub fn abs_f32(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFF_FFFF)
}

#[inline]
pub fn floor_f32(x: f32) -> f32 {
    if x.is_nan() || x >= 8388608.0 || x <= -8388608.0 {
        return x;
    }
    let i = x as i32;
    let fi = i as f32;
    if x < fi {
        fi - 1.0
    } else {
        fi
    }
}

#[inline]
pub fn ceil_f32(x: f32) -> f32 {
    -floor_f32(-x)
}

#[inline]
pub fn round_f32(x: f32) -> f32 {
    floor_f32(x + 0.5)
}

/// Square root via IEEE-754 bit-level reciprocal seed + 3 Newton-Raphson iterations.
#[inline]
pub fn sqrt_f32(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x.is_infinite() {
        return f32::INFINITY;
    }
    let bits = x.to_bits();
    let mut y = f32::from_bits((bits >> 1) + 0x1FBB4000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    y
}

/// Sine using range reduction to [-pi/2, pi/2] and minimax polynomial approximation.
#[inline]
pub fn sin_f32(mut x: f32) -> f32 {
    if x.is_nan() || x.is_infinite() {
        return f32::NAN;
    }
    // Reduce modulo TAU
    let div = x / TAU;
    x -= round_f32(div) * TAU;

    // Reduce to [-PI/2, PI/2]
    if x > FRAC_PI_2 {
        x = PI - x;
    } else if x < -FRAC_PI_2 {
        x = -PI - x;
    }

    let z = x * x;
    // Minimax polynomial for sin(x) on [-PI/2, PI/2]
    x * (0.99999994 + z * (-0.16666652 + z * (0.008333026 + z * (-0.000198074 + z * 0.000002602))))
}

#[inline]
pub fn cos_f32(x: f32) -> f32 {
    sin_f32(x + FRAC_PI_2)
}

#[inline]
pub fn sin_cos_f32(x: f32) -> (f32, f32) {
    (sin_f32(x), cos_f32(x))
}

#[inline]
pub fn tan_f32(x: f32) -> f32 {
    let c = cos_f32(x);
    if abs_f32(c) < 1e-12 {
        return if sin_f32(x) >= 0.0 { 1e12 } else { -1e12 };
    }
    sin_f32(x) / c
}

/// Base-2 exponential 2^x.
#[inline]
pub fn exp2_f32(x: f32) -> f32 {
    if x.is_nan() {
        return f32::NAN;
    }
    if x <= -126.0 {
        return 0.0;
    }
    if x >= 127.0 {
        return f32::INFINITY;
    }
    let fl = floor_f32(x);
    let i = fl as i32;
    let f = x - fl;
    // Minimax polynomial for 2^f on [0, 1)
    let p = 1.0 + f * (0.6931472 + f * (0.2402265 + f * (0.0555041 + f * (0.0096181 + f * 0.0013334))));
    let scale = f32::from_bits(((i + 127) as u32) << 23);
    p * scale
}

/// Natural exponential e^x = 2^(x * log2(e)).
#[inline]
pub fn exp_f32(x: f32) -> f32 {
    exp2_f32(x * LOG2_E)
}

/// Base-2 logarithm log2(x).
#[inline]
pub fn log2_f32(x: f32) -> f32 {
    if x.is_nan() || x < 0.0 {
        return f32::NAN;
    }
    if x == 0.0 {
        return -f32::INFINITY;
    }
    if x.is_infinite() {
        return f32::INFINITY;
    }
    let bx = x.to_bits();
    let exp = ((bx >> 23) as i32) - 127;
    let mant = f32::from_bits((bx & 0x007F_FFFF) | 0x3F80_0000);
    // mant in [1.0, 2.0). Map to u in [0, 1/3)
    let u = (mant - 1.0) / (mant + 1.0);
    let u2 = u * u;
    let ln_mant = 2.0 * u * (1.0 + u2 * (0.33333333 + u2 * (0.2 + u2 * (0.14285714 + u2 * 0.11111111))));
    (exp as f32) + ln_mant * LOG2_E
}

/// Natural logarithm ln(x).
#[inline]
pub fn ln_f32(x: f32) -> f32 {
    log2_f32(x) * LN_2
}

/// Base-10 logarithm log10(x).
#[inline]
pub fn log10_f32(x: f32) -> f32 {
    log2_f32(x) * LOG10_2
}

/// Power base^exp.
#[inline]
pub fn powf_f32(base: f32, exp: f32) -> f32 {
    if exp == 0.0 {
        return 1.0;
    }
    if base == 0.0 {
        return if exp > 0.0 { 0.0 } else { f32::INFINITY };
    }
    if base < 0.0 {
        let int_exp = round_f32(exp);
        if abs_f32(exp - int_exp) < 1e-6 {
            let p = exp2_f32(exp * log2_f32(abs_f32(base)));
            return if (int_exp as i32) % 2 != 0 { -p } else { p };
        }
        return f32::NAN;
    }
    exp2_f32(exp * log2_f32(base))
}
