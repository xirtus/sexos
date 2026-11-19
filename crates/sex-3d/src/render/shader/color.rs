// SPDX-License-Identifier: GPL-3.0-only
//! Colour helpers of the shader VM, after Cycles' `util/color.h` and
//! `kernel/svm/color_util.h`: the Mix node's blend modes, HSV/HSL, the
//! Blackbody and Wavelength nodes.
pub type Rgb = [f32; 3];

pub fn rgb_to_hsv(c: Rgb) -> Rgb {
    let cmax = c[0].max(c[1]).max(c[2]);
    let cmin = c[0].min(c[1]).min(c[2]);
    let cdelta = cmax - cmin;
    let v = cmax;
    let s = if cmax != 0.0 { cdelta / cmax } else { 0.0 };
    let mut h = 0.0;
    if s != 0.0 {
        let cc = [(cmax - c[0]) / cdelta, (cmax - c[1]) / cdelta, (cmax - c[2]) / cdelta];
        h = if c[0] == cmax {
            cc[2] - cc[1]
        } else if c[1] == cmax {
            2.0 + cc[0] - cc[2]
        } else {
            4.0 + cc[1] - cc[0]
        };
        h /= 6.0;
        if h < 0.0 {
            h += 1.0;
        }
    }
    [h, s, v]
}

pub fn hsv_to_rgb(hsv: Rgb) -> Rgb {
    let [mut h, s, v] = hsv;
    if s == 0.0 {
        return [v, v, v];
    }
    if h == 1.0 {
        h = 0.0;
    }
    h *= 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

pub fn rgb_to_hsl(c: Rgb) -> Rgb {
    let cmax = c[0].max(c[1]).max(c[2]);
    let cmin = c[0].min(c[1]).min(c[2]);
    let l = ((cmax + cmin) / 2.0).min(1.0);
    if cmax == cmin {
        return [0.0, 0.0, l];
    }
    let d = cmax - cmin;
    let s = if l > 0.5 { d / (2.0 - cmax - cmin) } else { d / (cmax + cmin) };
    let h = if cmax == c[0] {
        (c[1] - c[2]) / d + if c[1] < c[2] { 6.0 } else { 0.0 }
    } else if cmax == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    [h / 6.0, s, l]
}

pub fn hsl_to_rgb(hsl: Rgb) -> Rgb {
    let [h, s, l] = hsl;
    let nr = ((h * 6.0 - 3.0).abs() - 1.0).clamp(0.0, 1.0);
    let ng = (2.0 - (h * 6.0 - 2.0).abs()).clamp(0.0, 1.0);
    let nb = (2.0 - (h * 6.0 - 4.0).abs()).clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    [(nr - 0.5) * chroma + l, (ng - 0.5) * chroma + l, (nb - 0.5) * chroma + l]
}

/// Rec. 709 luminance (`linear_rgb_to_gray` with Blender's default scene).
pub fn luminance(c: Rgb) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// The Mix node's colour blend modes, by RNA identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Mix,
    Darken,
    Multiply,
    Burn,
    Lighten,
    Screen,
    Dodge,
    Add,
    Overlay,
    SoftLight,
    LinearLight,
    Difference,
    Exclusion,
    Subtract,
    Divide,
    Hue,
    Saturation,
    Color,
    Value,
}

impl Blend {
    pub fn from_name(s: &str) -> Blend {
        match s {
            "DARKEN" => Blend::Darken,
            "MULTIPLY" => Blend::Multiply,
            "BURN" => Blend::Burn,
            "LIGHTEN" => Blend::Lighten,
            "SCREEN" => Blend::Screen,
            "DODGE" => Blend::Dodge,
            "ADD" => Blend::Add,
            "OVERLAY" => Blend::Overlay,
            "SOFT_LIGHT" => Blend::SoftLight,
            "LINEAR_LIGHT" => Blend::LinearLight,
            "DIFFERENCE" => Blend::Difference,
            "EXCLUSION" => Blend::Exclusion,
            "SUBTRACT" => Blend::Subtract,
            "DIVIDE" => Blend::Divide,
            "HUE" => Blend::Hue,
            "SATURATION" => Blend::Saturation,
            "COLOR" => Blend::Color,
            "VALUE" => Blend::Value,
            _ => Blend::Mix,
        }
    }

    /// `MA_RAMP_*` numbers stored in files (`ShaderNodeMixRGB.custom1`,
    /// `NodeShaderMix.blend_type`).
    pub fn from_file(n: i64) -> &'static str {
        const NAMES: [&str; 19] = [
            "MIX", "ADD", "MULTIPLY", "SUBTRACT", "SCREEN", "DIVIDE", "DIFFERENCE", "DARKEN", "LIGHTEN", "OVERLAY", "DODGE", "BURN", "HUE", "SATURATION", "VALUE", "COLOR",
            "SOFT_LIGHT", "LINEAR_LIGHT", "EXCLUSION",
        ];
        NAMES.get(n as usize).copied().unwrap_or("MIX")
    }
}

fn lerp3(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn map3(a: Rgb, b: Rgb, f: impl Fn(f32, f32) -> f32) -> Rgb {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// `svm_mix`: blend `c2` over `c1` by `t` (not clamped here).
pub fn blend(mode: Blend, t: f32, c1: Rgb, c2: Rgb) -> Rgb {
    let tm = 1.0 - t;
    match mode {
        Blend::Mix => lerp3(c1, c2, t),
        Blend::Add => lerp3(c1, map3(c1, c2, |a, b| a + b), t),
        Blend::Multiply => lerp3(c1, map3(c1, c2, |a, b| a * b), t),
        Blend::Screen => map3(c1, c2, |a, b| 1.0 - (tm + t * (1.0 - b)) * (1.0 - a)),
        Blend::Overlay => map3(c1, c2, |a, b| if a < 0.5 { a * (tm + 2.0 * t * b) } else { 1.0 - (tm + 2.0 * t * (1.0 - b)) * (1.0 - a) }),
        Blend::Subtract => lerp3(c1, map3(c1, c2, |a, b| a - b), t),
        Blend::Divide => map3(c1, c2, |a, b| if b != 0.0 { tm * a + t * a / b } else { a }),
        Blend::Difference => lerp3(c1, map3(c1, c2, |a, b| (a - b).abs()), t),
        Blend::Exclusion => lerp3(c1, map3(c1, c2, |a, b| a + b - 2.0 * a * b), t).map(|x| x.max(0.0)),
        Blend::Darken => lerp3(c1, map3(c1, c2, f32::min), t),
        Blend::Lighten => lerp3(c1, map3(c1, c2, f32::max), t),
        Blend::Dodge => map3(c1, c2, |a, b| {
            if a == 0.0 {
                return a;
            }
            let tmp = 1.0 - t * b;
            if tmp <= 0.0 {
                1.0
            } else {
                (a / tmp).min(1.0)
            }
        }),
        Blend::Burn => map3(c1, c2, |a, b| {
            let tmp = tm + t * b;
            if tmp <= 0.0 {
                0.0
            } else {
                (1.0 - (1.0 - a) / tmp).clamp(0.0, 1.0)
            }
        }),
        Blend::Hue => {
            let h2 = rgb_to_hsv(c2);
            if h2[1] != 0.0 {
                let mut h = rgb_to_hsv(c1);
                h[0] = h2[0];
                lerp3(c1, hsv_to_rgb(h), t)
            } else {
                c1
            }
        }
        Blend::Saturation => {
            let mut h = rgb_to_hsv(c1);
            if h[1] != 0.0 {
                let h2 = rgb_to_hsv(c2);
                h[1] = tm * h[1] + t * h2[1];
                hsv_to_rgb(h)
            } else {
                c1
            }
        }
        Blend::Value => {
            let mut h = rgb_to_hsv(c1);
            let h2 = rgb_to_hsv(c2);
            h[2] = tm * h[2] + t * h2[2];
            hsv_to_rgb(h)
        }
        Blend::Color => {
            let h2 = rgb_to_hsv(c2);
            if h2[1] != 0.0 {
                let mut h = rgb_to_hsv(c1);
                h[0] = h2[0];
                h[1] = h2[1];
                lerp3(c1, hsv_to_rgb(h), t)
            } else {
                c1
            }
        }
        Blend::SoftLight => map3(c1, c2, |a, b| {
            let scr = 1.0 - (1.0 - b) * (1.0 - a);
            tm * a + t * ((1.0 - a) * b * a + a * scr)
        }),
        Blend::LinearLight => map3(c1, c2, |a, b| a + t * (2.0 * b - 1.0)),
    }
}

/// The CIE 1931 2° colour matching functions, as the multi-lobe Gaussian
/// fit of Wyman, Sloan and Shirley (2013) (within about 1% of the tables).
pub fn cie_xyz(lambda_nm: f32) -> Rgb {
    let g = |x: f32, mu: f32, s1: f32, s2: f32| {
        let s = if x < mu { s1 } else { s2 };
        let t = (x - mu) / s;
        (-0.5 * t * t).exp()
    };
    let l = lambda_nm;
    [
        1.056 * g(l, 599.8, 37.9, 31.0) + 0.362 * g(l, 442.0, 16.0, 26.7) - 0.065 * g(l, 501.1, 20.4, 26.2),
        0.821 * g(l, 568.8, 46.9, 40.5) + 0.286 * g(l, 530.9, 16.3, 31.1),
        1.217 * g(l, 437.0, 11.8, 36.0) + 0.681 * g(l, 459.0, 26.0, 13.8),
    ]
}

/// CIE XYZ to linear Rec. 709.
pub fn xyz_to_rgb(c: Rgb) -> Rgb {
    [
        3.240_454_2 * c[0] - 1.537_138_5 * c[1] - 0.498_531_4 * c[2],
        -0.969_266 * c[0] + 1.876_010_8 * c[1] + 0.041_556 * c[2],
        0.055_643_4 * c[0] - 0.204_025_9 * c[1] + 1.057_225_2 * c[2],
    ]
}

/// Blackbody node: the colour of a black body at `t` kelvin in linear
/// Rec. 709, normalised to luminance 1 like Cycles (clamped to 800..12000 K;
/// may be out of gamut, i.e. negative, at the low end).
pub fn blackbody(t: f32) -> Rgb {
    let t = t.clamp(800.0, 12000.0) as f64;
    let mut xyz = [0.0f64; 3];
    let mut l = 380.0f64;
    while l <= 780.0 {
        let lm = l * 1e-9;
        // Planck's law up to a constant factor.
        let b = 1.0 / (lm.powi(5) * ((1.438_776_9e-2 / (lm * t)).exp() - 1.0));
        let c = cie_xyz(l as f32);
        for k in 0..3 {
            xyz[k] += b * c[k] as f64;
        }
        l += 5.0;
    }
    let rgb = xyz_to_rgb([xyz[0] as f32, xyz[1] as f32, xyz[2] as f32]);
    let y = luminance(rgb);
    if y > 0.0 {
        rgb.map(|v| v / y)
    } else {
        [0.0; 3]
    }
}

/// Wavelength node: the spectral colour of `lambda_nm`, scaled by 1/2.52 and
/// clamped to positive like Cycles' `svm_node_wavelength`.
pub fn wavelength(lambda_nm: f32) -> Rgb {
    if !(380.0..780.0).contains(&lambda_nm) {
        return [0.0; 3];
    }
    xyz_to_rgb(cie_xyz(lambda_nm)).map(|v| (v / 2.52).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close3(a: Rgb, b: Rgb, eps: f32) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() <= eps)
    }

    #[test]
    fn hsv_hsl_round_trip() {
        for c in [[0.2, 0.5, 0.9], [0.9, 0.1, 0.3], [0.4, 0.4, 0.4], [1.0, 0.8, 0.0]] {
            assert!(close3(hsv_to_rgb(rgb_to_hsv(c)), c, 1e-6), "{c:?}");
            assert!(close3(hsl_to_rgb(rgb_to_hsl(c)), c, 1e-6), "{c:?}");
        }
        assert!(close3(rgb_to_hsv([1.0, 0.0, 0.0]), [0.0, 1.0, 1.0], 1e-6));
        assert!(close3(rgb_to_hsv([0.0, 0.5, 0.0]), [1.0 / 3.0, 1.0, 0.5], 1e-6));
    }

    #[test]
    fn blend_modes() {
        let (a, b) = ([0.2, 0.4, 0.8], [0.5, 0.5, 0.25]);
        assert!(close3(blend(Blend::Mix, 0.5, a, b), [0.35, 0.45, 0.525], 1e-6));
        assert!(close3(blend(Blend::Add, 1.0, a, b), [0.7, 0.9, 1.05], 1e-6));
        assert!(close3(blend(Blend::Multiply, 1.0, a, b), [0.1, 0.2, 0.2], 1e-6));
        assert!(close3(blend(Blend::Subtract, 1.0, a, b), [-0.3, -0.1, 0.55], 1e-6));
        assert!(close3(blend(Blend::Screen, 1.0, a, b), [0.6, 0.7, 0.85], 1e-6));
        assert!(close3(blend(Blend::Divide, 1.0, a, b), [0.4, 0.8, 3.2], 1e-6));
        assert!(close3(blend(Blend::Difference, 1.0, a, b), [0.3, 0.1, 0.55], 1e-6));
        assert!(close3(blend(Blend::Darken, 1.0, a, b), [0.2, 0.4, 0.25], 1e-6));
        assert!(close3(blend(Blend::Lighten, 1.0, a, b), [0.5, 0.5, 0.8], 1e-6));
        // Overlay: dark base multiplies (2ab), light base screens.
        assert!(close3(blend(Blend::Overlay, 1.0, a, b), [0.2, 0.4, 1.0 - 2.0 * 0.75 * 0.2], 1e-6));
        assert!(close3(blend(Blend::Exclusion, 1.0, a, b), [0.5, 0.5, 0.65], 1e-6));
        assert!(close3(blend(Blend::LinearLight, 1.0, a, b), [0.2, 0.4, 0.3], 1e-6));
        assert!(close3(blend(Blend::Dodge, 1.0, a, b), [0.4, 0.8, 1.0], 1e-6));
        assert!(close3(blend(Blend::Burn, 1.0, a, b), [0.0, 0.0, 0.2], 1e-6));
        let sl = blend(Blend::SoftLight, 1.0, a, b);
        assert!((sl[0] - (0.8 * 0.5 * 0.2 + 0.2 * (1.0 - 0.5 * 0.8))).abs() < 1e-6);
        // Value: keeps hue and saturation of A, takes B's value.
        let v = blend(Blend::Value, 1.0, a, b);
        assert!((rgb_to_hsv(v)[2] - 0.5).abs() < 1e-6);
        let h = blend(Blend::Hue, 1.0, [0.5, 0.5, 0.5], [1.0, 0.0, 0.0]);
        assert!(close3(h, [0.5, 0.5, 0.5], 1e-6)); // grey has no saturation to keep
        let c = blend(Blend::Color, 1.0, [0.5, 0.25, 0.25], [0.0, 0.0, 1.0]);
        assert!((rgb_to_hsv(c)[0] - 2.0 / 3.0).abs() < 1e-5);
        for (i, m) in ["MIX", "ADD", "MULTIPLY", "SUBTRACT"].iter().enumerate() {
            assert_eq!(Blend::from_file(i as i64), *m);
        }
    }

    #[test]
    fn blackbody_matches_cycles() {
        // Cycles' fitted values at its table ends (12000 K and 800 K).
        assert!(close3(blackbody(12000.0), [0.826, 0.995, 1.566], 0.03), "{:?}", blackbody(12000.0));
        let c = blackbody(800.0);
        assert!((c[0] - 5.41).abs() < 0.6 && c[1] < 0.0, "{c:?}");
        let d = blackbody(6500.0);
        assert!((luminance(d) - 1.0).abs() < 1e-4 && (d[0] - d[2]).abs() < 0.15, "{d:?}");
        // Monochromatic green is green.
        let w = wavelength(530.0);
        assert!(w[1] > w[0] && w[1] > w[2] && w[1] < 1.0);
        assert_eq!(wavelength(300.0), [0.0; 3]);
    }
}
