// SPDX-License-Identifier: GPL-3.0-only
//! Colour management: scene-linear [`Image`] to display sRGB bytes through a
//! view transform (Blender's list), a look, exposure and gamma.
use crate::scene::Image;

/// How scene-linear colour is mapped to the display (Blender's "View Transform").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewTransform {
    /// Plain sRGB transfer, clipping at 1.
    #[default]
    Standard,
    /// Per-channel log encoding with a filmic S-curve (an approximation of
    /// Blender's Filmic: 16.5 stops around middle grey).
    Filmic,
    /// AgX-style log encoding plus a sigmoid: highlights roll off and
    /// desaturate gracefully instead of clipping.
    AgX,
    /// The Khronos PBR Neutral tone mapper (keeps base colours exact up to 0.8).
    KhronosPbrNeutral,
    /// Exposure bands in false colours, for judging lighting.
    FalseColor,
    /// No transform at all: linear values written as they are.
    Raw,
}

impl ViewTransform {
    pub const ALL: [ViewTransform; 6] = [ViewTransform::Standard, ViewTransform::KhronosPbrNeutral, ViewTransform::AgX, ViewTransform::Filmic, ViewTransform::FalseColor, ViewTransform::Raw];
    pub fn label(self) -> &'static str {
        match self {
            ViewTransform::Standard => "Standard",
            ViewTransform::Filmic => "Filmic",
            ViewTransform::AgX => "AgX",
            ViewTransform::KhronosPbrNeutral => "Khronos PBR Neutral",
            ViewTransform::FalseColor => "False Color",
            ViewTransform::Raw => "Raw",
        }
    }
    pub fn from_label(s: &str) -> Option<ViewTransform> {
        ViewTransform::ALL.into_iter().find(|v| v.label() == s)
    }
}

/// Looks (Blender's AgX and Filmic look list), applied with the view transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    None,
    Punchy,
    Greyscale,
    VeryHighContrast,
    HighContrast,
    MediumHighContrast,
    BaseContrast,
    MediumLowContrast,
    LowContrast,
    VeryLowContrast,
}

impl Look {
    pub const ALL: [Look; 10] = [
        Look::None,
        Look::Punchy,
        Look::Greyscale,
        Look::VeryHighContrast,
        Look::HighContrast,
        Look::MediumHighContrast,
        Look::BaseContrast,
        Look::MediumLowContrast,
        Look::LowContrast,
        Look::VeryLowContrast,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Look::None => "None",
            Look::Punchy => "Punchy",
            Look::Greyscale => "Greyscale",
            Look::VeryHighContrast => "Very High Contrast",
            Look::HighContrast => "High Contrast",
            Look::MediumHighContrast => "Medium High Contrast",
            Look::BaseContrast => "Base Contrast",
            Look::MediumLowContrast => "Medium Low Contrast",
            Look::LowContrast => "Low Contrast",
            Look::VeryLowContrast => "Very Low Contrast",
        }
    }
    pub fn from_label(s: &str) -> Option<Look> {
        Look::ALL.into_iter().find(|v| v.label() == s)
    }
    /// Contrast scale around middle grey in log space.
    fn contrast(self) -> f32 {
        match self {
            Look::VeryHighContrast => 1.57,
            Look::HighContrast => 1.4,
            Look::MediumHighContrast => 1.2,
            Look::MediumLowContrast => 0.9,
            Look::LowContrast => 0.8,
            Look::VeryLowContrast => 0.7,
            _ => 1.0,
        }
    }
}

/// Blender's Color Management panel (display device sRGB).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorManagement {
    pub view: ViewTransform,
    pub look: Look,
    /// Stops.
    pub exposure: f32,
    pub gamma: f32,
}

impl Default for ColorManagement {
    fn default() -> Self {
        ColorManagement { view: ViewTransform::AgX, look: Look::None, exposure: 0.0, gamma: 1.0 }
    }
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
    to_display8(img, &ColorManagement { view, look: Look::None, exposure, gamma: 1.0 })
}

/// RGBA8 through the full colour management. Alpha is kept linear.
pub fn to_display8(img: &Image, cm: &ColorManagement) -> Vec<u8> {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut out = Vec::with_capacity(img.pixels.len() * 4);
    for p in &img.pixels {
        let d = display(cm, [p[0], p[1], p[2]]);
        out.extend_from_slice(&[byte(d[0]), byte(d[1]), byte(d[2]), byte(p[3])]);
    }
    out
}

fn lum(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// One scene-linear colour to display-encoded 0..1.
pub fn display(cm: &ColorManagement, rgb: [f32; 3]) -> [f32; 3] {
    let gain = if cm.exposure.is_finite() { cm.exposure.exp2() } else { 1.0 };
    let mut rgb = rgb.map(|c| if c.is_finite() { (c * gain).max(0.0) } else { 0.0 });
    // Contrast looks: scale log exposure around middle grey.
    let k = cm.look.contrast();
    if k != 1.0 && cm.view != ViewTransform::Raw && cm.view != ViewTransform::FalseColor {
        rgb = rgb.map(|c| if c > 0.0 { 0.18 * (c / 0.18).powf(k) } else { 0.0 });
    }
    let mut d = match cm.view {
        ViewTransform::Standard => rgb.map(srgb_encode),
        ViewTransform::Filmic => rgb.map(|c| sigmoid(log_encode(c))),
        ViewTransform::AgX => agx(rgb),
        ViewTransform::KhronosPbrNeutral => pbr_neutral(rgb).map(srgb_encode),
        ViewTransform::FalseColor => false_color(lum(rgb)),
        ViewTransform::Raw => rgb,
    };
    match cm.look {
        Look::Punchy if cm.view != ViewTransform::Raw => {
            // ASC CDL power 1.35, saturation 1.4 (Blender's AgX Punchy).
            d = d.map(|c| c.clamp(0.0, 1.0).powf(1.35));
            let l = lum(d);
            d = d.map(|c| l + (c - l) * 1.4);
        }
        Look::Greyscale => {
            let l = lum(d);
            d = [l; 3];
        }
        _ => {}
    }
    if cm.gamma.is_finite() && cm.gamma > 0.0 && cm.gamma != 1.0 {
        d = d.map(|c| c.clamp(0.0, 1.0).powf(1.0 / cm.gamma));
    }
    d
}

/// Log2 encoding over [-12.47, 4.03] EV (16.5 stops, middle grey at 0.606).
fn log_encode(c: f32) -> f32 {
    const MIN_EV: f32 = -12.473_93;
    const MAX_EV: f32 = 4.026_069;
    ((c.max(1e-10).log2() - MIN_EV) / (MAX_EV - MIN_EV)).clamp(0.0, 1.0)
}

/// The AgX base contrast sigmoid (output display-encoded).
fn sigmoid(x: f32) -> f32 {
    let x2 = x * x;
    let x4 = x2 * x2;
    15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.002_32
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

/// AgX: inset, log2 encode, sigmoid, outset. The sigmoid output is already
/// display-encoded.
fn agx(rgb: [f32; 3]) -> [f32; 3] {
    let v = mul(&AGX_IN, rgb.map(|c| if c.is_finite() { c.max(0.0) } else { 0.0 }));
    mul(&AGX_OUT, v.map(|c| sigmoid(log_encode(c))))
}

/// Khronos PBR Neutral (linear in, linear out).
fn pbr_neutral(c: [f32; 3]) -> [f32; 3] {
    const START: f32 = 0.8 - 0.04;
    const DESAT: f32 = 0.15;
    let x = c[0].min(c[1]).min(c[2]);
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    let c = c.map(|v| v - offset);
    let peak = c[0].max(c[1]).max(c[2]);
    if peak < START {
        return c;
    }
    let d = 1.0 - START;
    let new_peak = 1.0 - d * d / (peak + d - START);
    let c = c.map(|v| v * new_peak / peak);
    let g = 1.0 - 1.0 / (DESAT * (peak - new_peak) + 1.0);
    c.map(|v| v * (1.0 - g) + new_peak * g)
}

/// Exposure bands relative to middle grey (0.18), as display colours.
fn false_color(l: f32) -> [f32; 3] {
    let ev = if l > 0.0 { (l / 0.18).log2() } else { f32::NEG_INFINITY };
    const BANDS: [(f32, [f32; 3]); 9] = [
        (-10.0, [0.0, 0.0, 0.0]),
        (-7.0, [0.25, 0.0, 0.4]),
        (-4.0, [0.0, 0.0, 0.9]),
        (-2.0, [0.0, 0.6, 0.9]),
        (-0.5, [0.0, 0.7, 0.2]),
        (0.5, [0.5, 0.5, 0.5]),
        (2.0, [0.8, 0.85, 0.0]),
        (4.0, [1.0, 0.5, 0.0]),
        (6.5, [1.0, 0.0, 0.0]),
    ];
    for (top, col) in BANDS {
        if ev < top {
            return col;
        }
    }
    [1.0, 1.0, 1.0]
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

    #[test]
    fn view_transforms_behave() {
        let cm = |view| ColorManagement { view, look: Look::None, exposure: 0.0, gamma: 1.0 };
        // Raw passes linear values through.
        assert_eq!(display(&cm(ViewTransform::Raw), [0.25, 0.5, 2.0]), [0.25, 0.5, 2.0]);
        // PBR Neutral keeps mid colours (offset aside) and compresses highlights below 1.
        let n = pbr_neutral([0.5, 0.3, 0.2]);
        assert!((n[0] - 0.46).abs() < 1e-4);
        let hi = pbr_neutral([20.0, 20.0, 20.0]);
        assert!(hi[0] < 1.0 && hi[0] > 0.95);
        // Filmic and AgX map middle grey near the middle of the display range.
        for v in [ViewTransform::Filmic, ViewTransform::AgX] {
            let g = display(&cm(v), [0.18; 3])[1];
            assert!((0.35..0.65).contains(&g), "{v:?}: {g}");
            assert!(display(&cm(v), [1000.0; 3])[1] > 0.95);
        }
        // False colour: middle grey is grey, overexposure is white.
        assert_eq!(display(&cm(ViewTransform::FalseColor), [0.18; 3]), [0.5, 0.5, 0.5]);
        assert_eq!(display(&cm(ViewTransform::FalseColor), [1e4; 3]), [1.0, 1.0, 1.0]);
        // Exposure +1 doubles before the transform; gamma brightens.
        let a = display(&ColorManagement { exposure: 1.0, ..cm(ViewTransform::Raw) }, [0.25; 3]);
        assert_eq!(a, [0.5; 3]);
        let g = display(&ColorManagement { gamma: 2.0, ..cm(ViewTransform::Standard) }, [0.2; 3])[0];
        assert!(g > srgb_encode(0.2));
        // Looks: more contrast darkens shadows; greyscale has no chroma.
        let base = display(&cm(ViewTransform::AgX), [0.05; 3])[0];
        let hc = display(&ColorManagement { look: Look::HighContrast, ..cm(ViewTransform::AgX) }, [0.05; 3])[0];
        assert!(hc < base);
        let gr = display(&ColorManagement { look: Look::Greyscale, ..cm(ViewTransform::Standard) }, [0.8, 0.1, 0.1]);
        assert!((gr[0] - gr[1]).abs() < 1e-6 && (gr[1] - gr[2]).abs() < 1e-6);
    }
}
