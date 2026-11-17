// SPDX-License-Identifier: GPL-3.0-only
//! Lightweight `#![no_std]` math primitives and Eurorack volt/octave conversions.

pub const PI: f32 = 3.14159265358979323846;
pub const TWO_PI: f32 = 6.28318530717958647692;
pub const HALF_PI: f32 = 1.57079632679489661923;

pub const C4_HZ: f32 = 261.625565;
pub const A4_HZ: f32 = 440.0;

#[inline]
pub fn abs(x: f32) -> f32 {
    f32::from_bits(x.to_bits() & 0x7FFF_FFFF)
}

#[inline]
pub fn clamp(x: f32, min_val: f32, max_val: f32) -> f32 {
    if x < min_val {
        min_val
    } else if x > max_val {
        max_val
    } else {
        x
    }
}

#[inline]
pub fn floor(x: f32) -> f32 {
    if x >= 0.0 {
        (x as i32) as f32
    } else {
        let i = x as i32;
        let fi = i as f32;
        if fi == x {
            x
        } else {
            (i - 1) as f32
        }
    }
}

#[inline]
pub fn exp2(x: f32) -> f32 {
    if x.is_nan() {
        return 0.0;
    }
    if x < -126.0 {
        return 0.0;
    }
    if x > 127.0 {
        return f32::MAX;
    }
    let n = floor(x) as i32;
    let f = x - (n as f32);

    // Degree-5 polynomial for 2^f on [0, 1)
    const C1: f32 = 0.69314718056;
    const C2: f32 = 0.24022650696;
    const C3: f32 = 0.05550410866;
    const C4: f32 = 0.00961812911;
    const C5: f32 = 0.00133335581;

    let p = 1.0 + f * (C1 + f * (C2 + f * (C3 + f * (C4 + f * C5))));
    let scale = f32::from_bits(((n + 127) as u32) << 23);
    p * scale
}

#[inline]
pub fn sin(x: f32) -> f32 {
    if x.is_nan() || x.is_infinite() {
        return 0.0;
    }
    // Modulo 2*pi
    let div = x / TWO_PI;
    let mut y = x - floor(div) * TWO_PI;
    if y > PI {
        y -= TWO_PI;
    } else if y < -PI {
        y += TWO_PI;
    }

    if y > HALF_PI {
        y = PI - y;
    } else if y < -HALF_PI {
        y = -PI - y;
    }

    let y2 = y * y;
    y * (1.0 - y2 * (1.0 / 6.0 - y2 * (1.0 / 120.0 - y2 * (1.0 / 5040.0 - y2 * (1.0 / 362880.0)))))
}

#[inline]
pub fn cos(x: f32) -> f32 {
    sin(x + HALF_PI)
}

#[inline]
pub fn tan(x: f32) -> f32 {
    let c = cos(x);
    if abs(c) < 1.0e-7 {
        if sin(x) >= 0.0 {
            1.0e7
        } else {
            -1.0e7
        }
    } else {
        sin(x) / c
    }
}

#[inline]
pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 || x.is_nan() {
        return 0.0;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1FC00000);
    y = 0.5 * (y + x / y);
    y = 0.5 * (y + x / y);
    0.5 * (y + x / y)
}

#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.is_nan() {
        return 0.0;
    }
    if x > 3.0 {
        1.0
    } else if x < -3.0 {
        -1.0
    } else {
        let x2 = x * x;
        x * (27.0 + x2) / (27.0 + 9.0 * x2)
    }
}

#[inline]
pub fn tanh(x: f32) -> f32 {
    soft_clip(x)
}

#[inline]
pub fn cv_to_freq(pitch_cv: f32) -> f32 {
    // 0.0V = C4 = 261.6256 Hz (1V / octave)
    C4_HZ * exp2(pitch_cv)
}

#[inline]
pub fn cv_to_freq_a4(pitch_cv: f32) -> f32 {
    // 0.0V = A4 = 440.0 Hz
    A4_HZ * exp2(pitch_cv)
}

#[inline]
pub fn midi_to_cv(key: u8) -> f32 {
    (key as f32 - 60.0) / 12.0
}

#[inline]
pub fn midi_to_freq(key: u8) -> f32 {
    cv_to_freq(midi_to_cv(key))
}

#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    if t < dt {
        let ratio = t / dt;
        ratio + ratio - ratio * ratio - 1.0
    } else if t > 1.0 - dt {
        let ratio = (t - 1.0) / dt;
        ratio * ratio + ratio + ratio + 1.0
    } else {
        0.0
    }
}
