//! Pure-Rust `no_std` mathematical, transfer-function, and color-space helpers.

use core::f32::consts::PI;

#[inline]
pub(crate) fn absf(x: f32) -> f32 {
    libm::fabsf(x)
}

#[inline]
pub(crate) fn floorf(x: f32) -> f32 {
    libm::floorf(x)
}

#[inline]
pub(crate) fn roundf(x: f32) -> f32 {
    libm::roundf(x)
}

#[inline]
pub(crate) fn sqrtf(x: f32) -> f32 {
    libm::sqrtf(x.max(0.0))
}

#[inline]
pub(crate) fn sqrt(x: f64) -> f64 {
    libm::sqrt(x.max(0.0))
}

#[inline]
pub(crate) fn sinf(x: f32) -> f32 {
    libm::sinf(x)
}

#[inline]
pub(crate) fn cosf(x: f32) -> f32 {
    libm::cosf(x)
}

#[inline]
pub(crate) fn sin(x: f64) -> f64 {
    libm::sin(x)
}

#[inline]
pub(crate) fn cos(x: f64) -> f64 {
    libm::cos(x)
}

#[inline]
pub(crate) fn atan2f(y: f32, x: f32) -> f32 {
    libm::atan2f(y, x)
}

#[inline]
pub(crate) fn powf(base: f32, exp: f32) -> f32 {
    libm::powf(base, exp)
}

#[inline]
pub(crate) fn pow(base: f64, exp: f64) -> f64 {
    libm::pow(base, exp)
}

#[inline]
pub(crate) fn expf(x: f32) -> f32 {
    libm::expf(x)
}

#[inline]
pub(crate) fn exp(x: f64) -> f64 {
    libm::exp(x)
}

#[inline]
pub(crate) fn log10f(x: f32) -> f32 {
    libm::log10f(x)
}

#[inline]
pub(crate) fn log10(x: f64) -> f64 {
    libm::log10(x)
}

#[inline]
pub(crate) fn clamp01(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[inline]
pub(crate) fn fract(x: f32) -> f32 {
    x - floorf(x)
}

#[inline]
pub(crate) fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let span = edge1 - edge0;
    if absf(span) < 1e-7 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = clamp01((x - edge0) / span);
    t * t * (3.0 - 2.0 * t)
}

/// Exact IEC 61966-2-1 sRGB to linear transfer function.
#[inline]
pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        powf((c + 0.055) / 1.055, 2.4)
    }
}

/// Exact IEC 61966-2-1 linear to sRGB transfer function.
#[inline]
pub(crate) fn linear_to_srgb(c: f32) -> f32 {
    let c = c.max(0.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * powf(c, 1.0 / 2.4) - 0.055
    }
}

#[inline]
pub(crate) fn srgb_u8_to_linear(v: u8) -> f32 {
    srgb_to_linear(v as f32 / 255.0)
}

#[inline]
pub(crate) fn linear_to_srgb_u8(c: f32) -> u8 {
    let s = clamp01(linear_to_srgb(c));
    roundf(s * 255.0).clamp(0.0, 255.0) as u8
}

#[inline]
pub(crate) fn rec709_luma(rgb: [f32; 3]) -> f32 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

/// Converts RGB `[0..1]` to ITU-R BT.709 YCbCr where `Y in [0,1]` and `Cb, Cr in [-0.5, 0.5]`.
#[inline]
pub(crate) fn rgb_to_ycbcr_709(rgb: [f32; 3]) -> [f32; 3] {
    let r = rgb[0];
    let g = rgb[1];
    let b = rgb[2];
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let cb = (b - y) / 1.8556;
    let cr = (r - y) / 1.5748;
    [y, cb, cr]
}

#[inline]
pub(crate) fn db_to_linear(db: f32) -> f32 {
    powf(10.0, db * 0.05)
}

#[inline]
pub(crate) fn linear_to_db(lin: f32) -> f32 {
    20.0 * log10f(absf(lin).max(1e-12))
}

#[inline]
pub(crate) fn sinc(x: f32) -> f32 {
    let ax = absf(x);
    if ax < 1e-5 {
        1.0
    } else {
        let px = PI * x;
        sinf(px) / px
    }
}

#[inline]
pub(crate) fn lanczos3_weight(x: f32) -> f32 {
    let ax = absf(x);
    if ax >= 3.0 {
        0.0
    } else {
        sinc(x) * sinc(x / 3.0)
    }
}

/// Catmull-Rom cubic kernel (`a = -0.5`).
#[inline]
pub(crate) fn bicubic_catmull_rom(x: f32) -> f32 {
    let ax = absf(x);
    if ax < 1.0 {
        (1.5 * ax - 2.5) * ax * ax + 1.0
    } else if ax < 2.0 {
        ((-0.5 * ax + 2.5) * ax - 4.0) * ax + 2.0
    } else {
        0.0
    }
}

#[inline]
pub(crate) fn hash2d(x: i32, y: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(374_761_393) ^ (y as u32).wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16_777_215.0
}

#[inline]
pub(crate) fn hash2d_vec2(x: i32, y: i32) -> [f32; 2] {
    let a = hash2d(x, y);
    let b = hash2d(x.wrapping_add(57), y.wrapping_add(131));
    [a, b]
}

pub(crate) fn value_noise_2d(x: f32, y: f32) -> f32 {
    let xi = floorf(x) as i32;
    let yi = floorf(y) as i32;
    let xf = x - xi as f32;
    let yf = y - yi as f32;
    let u = xf * xf * (3.0 - 2.0 * xf);
    let v = yf * yf * (3.0 - 2.0 * yf);

    let n00 = hash2d(xi, yi);
    let n10 = hash2d(xi + 1, yi);
    let n01 = hash2d(xi, yi + 1);
    let n11 = hash2d(xi + 1, yi + 1);

    lerp(lerp(n00, n10, u), lerp(n01, n11, u), v)
}

pub(crate) fn fbm_2d(x: f32, y: f32, octaves: u8) -> f32 {
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for _ in 0..octaves {
        sum += amp * value_noise_2d(x * freq, y * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.0
    }
}
