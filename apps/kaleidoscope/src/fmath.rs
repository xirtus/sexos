// SPDX-License-Identifier: GPL-3.0-only
//! Elementary functions for the script subset's `Math` without libm: range
//! reduction to a small interval, then a polynomial. Results are within a
//! few units in the last place of the correctly rounded value (JavaScript
//! leaves the exact bits of these functions to each implementation).

use core::f64::consts::{FRAC_PI_2, LN_2, PI};

/// 2^k for an integer exponent in the normal range (0 or infinity outside it).
fn pow2(k: i64) -> f64 {
    if k > 1023 { f64::INFINITY } else if k < -1022 { if k < -1074 { 0.0 } else { f64::from_bits(1u64 << (k + 1074)) } } else { f64::from_bits(((k + 1023) as u64) << 52) }
}
fn round(x: f64) -> f64 { let t = x as i64 as f64; if (x - t).abs() >= 0.5 { t + x.signum() } else { t } }

pub fn exp(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x > 709.8 { return f64::INFINITY; }
    if x < -745.2 { return 0.0; }
    // x = k ln2 + r, |r| <= ln2/2, with ln2 split so k ln2 is exact.
    const LN2_HI: f64 = 6.93147180369123816490e-01;
    const LN2_LO: f64 = 1.90821492927058770002e-10;
    let k = round(x / LN_2);
    let r = (x - k * LN2_HI) - k * LN2_LO;
    let (mut term, mut sum) = (1.0f64, 1.0f64);
    for i in 1..=20 { term *= r / i as f64; sum += term; if term.abs() < 1e-18 { break; } }
    let k = k as i64;
    // Scale in two steps so 2^k never leaves the normal range on its own.
    if k > 1000 { sum * pow2(k - 1000) * pow2(1000) } else if k < -1000 { sum * pow2(k + 1000) * pow2(-1000) } else { sum * pow2(k) }
}

pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 { return f64::NAN; }
    if x == 0.0 { return f64::NEG_INFINITY; }
    if x.is_infinite() { return x; }
    if x == 1.0 { return 0.0; }
    // x = m 2^e with m in [sqrt(1/2), sqrt(2)); ln m = 2 atanh((m-1)/(m+1)).
    let (mut m, mut e) = (x, 0i64);
    if m < f64::MIN_POSITIVE { m *= pow2(54); e -= 54; }
    let bits = m.to_bits();
    e += ((bits >> 52) & 0x7ff) as i64 - 1023;
    m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if m > core::f64::consts::SQRT_2 { m /= 2.0; e += 1; }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let (mut term, mut sum, mut i) = (s, 0.0f64, 1.0f64);
    while i < 60.0 { sum += term / i; term *= s2; i += 2.0; if term.abs() < 1e-20 { break; } }
    e as f64 * LN_2 + 2.0 * sum
}

/// sin and cos of x reduced to [-pi/4, pi/4] by quarter turns.
pub fn sin_cos(x: f64) -> (f64, f64) {
    if !x.is_finite() { return (f64::NAN, f64::NAN); }
    if x == 0.0 { return (x, 1.0); }
    const PIO2_HI: f64 = 1.57079632673412561417e+00;
    const PIO2_LO: f64 = 6.07710050650619224932e-11;
    let q = round(x / FRAC_PI_2);
    let r = (x - q * PIO2_HI) - q * PIO2_LO;
    let r2 = r * r;
    let (mut st, mut s, mut ct, mut c) = (r, r, 1.0f64, 1.0f64);
    for i in 1..=12 {
        let n = (2 * i) as f64;
        st *= -r2 / (n * (n + 1.0)); s += st;
        ct *= -r2 / ((n - 1.0) * n); c += ct;
    }
    match (q as i64).rem_euclid(4) { 0 => (s, c), 1 => (c, -s), 2 => (-s, -c), _ => (-c, s) }
}

/// atan for any x: |x| > 1 uses pi/2 - atan(1/x); then two half-angle
/// steps make the series converge quickly.
pub fn atan(x: f64) -> f64 {
    if x.is_nan() { return x; }
    if x.is_infinite() { return FRAC_PI_2.copysign(x); }
    if x.abs() > 1.0 { return FRAC_PI_2.copysign(x) - atan(1.0 / x); }
    let mut t = x;
    for _ in 0..2 { t /= 1.0 + crate::script::sqrt_f64(1.0 + t * t); }
    let t2 = t * t;
    let (mut term, mut sum, mut i) = (t, 0.0f64, 1.0f64);
    while i < 80.0 { sum += term / i; term *= -t2; i += 2.0; if term.abs() < 1e-20 { break; } }
    4.0 * sum
}

pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() { return f64::NAN; }
    if x == 0.0 { return if y == 0.0 { if x.is_sign_negative() { PI.copysign(y) } else { y } } else { FRAC_PI_2.copysign(y) }; }
    if x.is_infinite() && y.is_infinite() { return (if x > 0.0 { PI / 4.0 } else { 3.0 * PI / 4.0 }).copysign(y); }
    let a = atan(y / x);
    if x > 0.0 { a } else if y >= 0.0 && !y.is_sign_negative() { a + PI } else { a - PI }
}

/// log10 / log2 exact at exact powers of the base (`log10(1000)` is 3).
pub fn log_base(x: f64, base: f64, ln_base: f64) -> f64 {
    let r = ln(x) / ln_base;
    let n = round(r);
    if r.is_finite() && (r - n).abs() < 1e-9 && n.abs() <= 300.0 {
        let mut p = 1.0f64;
        for _ in 0..n.abs() as i64 { p *= base; }
        if (if n < 0.0 { 1.0 / p } else { p }) == x { return n; }
    }
    r
}
