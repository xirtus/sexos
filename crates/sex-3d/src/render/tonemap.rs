// SPDX-License-Identifier: GPL-3.0-only
//! View transforms: scene-linear [`Image`] to display sRGB bytes.
use crate::scene::Image;

/// How scene-linear colour is mapped to the display (Blender's "View Transform").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewTransform {
    /// Plain sRGB transfer, clipping at 1.
    #[default]
    Standard,
    /// AgX-style log encoding plus a sigmoid: highlights roll off and
    /// desaturate gracefully instead of clipping.
    AgX,
}

/// sRGB transfer function (linear 0..1 to encoded 0..1).
pub fn srgb_encode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// Inverse sRGB transfer function.
pub fn srgb_decode(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// RGBA8 with the Standard view transform.
pub fn to_srgb8(img: &Image) -> Vec<u8> {
    to_srgb8_with(img, ViewTransform::Standard, 0.0)
}

/// RGBA8 with a view transform and an exposure in stops. Alpha is kept linear.
pub fn to_srgb8_with(img: &Image, view: ViewTransform, exposure: f32) -> Vec<u8> {
    let gain = exposure.exp2();
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut out = Vec::with_capacity(img.pixels.len() * 4);
    for p in &img.pixels {
        let rgb = [p[0] * gain, p[1] * gain, p[2] * gain];
        let d = match view {
            ViewTransform::Standard => rgb.map(srgb_encode),
            ViewTransform::AgX => agx(rgb),
        };
        out.extend_from_slice(&[byte(d[0]), byte(d[1]), byte(d[2]), byte(p[3])]);
    }
    out
}

/// Columns of the AgX inset matrix and its outset inverse (Rec. 709).
const AGX_IN: [[f32; 3]; 3] = [[0.842_479_06, 0.042_328_24, 0.042_375_655], [0.078_433_6, 0.878_468_64, 0.078_433_6], [0.079_223_745, 0.079_166_13, 0.879_142_97]];
const AGX_OUT: [[f32; 3]; 3] = [[1.196_879, -0.052_896_85, -0.052_971_635], [-0.098_020_88, 1.151_903_1, -0.098_043_45], [-0.099_029_74, -0.098_961_18, 1.151_073_7]];

fn mul(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    let mut o = [0.0; 3];
    for (c, col) in m.iter().enumerate() {
        for r in 0..3 {
            o[r] += col[r] * v[c];
        }
    }
    o
}

/// AgX: inset, log2 encode over [-12.47, 4.03] EV, sigmoid, outset. The
/// sigmoid output is already display-encoded.
fn agx(rgb: [f32; 3]) -> [f32; 3] {
    const MIN_EV: f32 = -12.473_93;
    const MAX_EV: f32 = 4.026_069;
    let v = mul(&AGX_IN, rgb.map(|c| if c.is_finite() { c.max(0.0) } else { 0.0 }));
    let v = v.map(|c| {
        let x = ((c.max(1e-10).log2() - MIN_EV) / (MAX_EV - MIN_EV)).clamp(0.0, 1.0);
        let x2 = x * x;
        let x4 = x2 * x2;
        15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.002_32
    });
    mul(&AGX_OUT, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_round_trips_and_agx_is_monotone() {
        for i in 0..=10 {
            let c = i as f32 / 10.0;
            assert!((srgb_decode(srgb_encode(c)) - c).abs() < 1e-5);
        }
        let mut last = -1.0;
        for i in 0..40 {
            let v = agx([0.001 * 1.4f32.powi(i); 3])[1];
            assert!(v >= last - 1e-6);
            last = v;
        }
        assert!(agx([0.0; 3])[0] < 0.01 && agx([100.0; 3])[0] > 0.95);
    }
}
