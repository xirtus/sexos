// SPDX-License-Identifier: GPL-3.0-only
//! Vertex Paint and Texture Paint: Blender's color blend modes
//! (`IMB_blend_color_float`), painting color attributes ([`vertex`]) and
//! painting images through UVs or directly in the Image Editor ([`texture`]).
pub mod texture;
pub mod vertex;

/// Color blend modes (Blender's brush `blend`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    #[default]
    Mix,
    Darken,
    Multiply,
    ColorBurn,
    Lighten,
    Screen,
    ColorDodge,
    Add,
    Overlay,
    Difference,
    Exclusion,
    Subtract,
    EraseAlpha,
    AddAlpha,
}

impl BlendMode {
    pub const ALL: [BlendMode; 14] = [
        BlendMode::Mix,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::Add,
        BlendMode::Overlay,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Subtract,
        BlendMode::EraseAlpha,
        BlendMode::AddAlpha,
    ];
    pub fn label(self) -> &'static str {
        match self {
            BlendMode::Mix => "Mix",
            BlendMode::Darken => "Darken",
            BlendMode::Multiply => "Multiply",
            BlendMode::ColorBurn => "Color Burn",
            BlendMode::Lighten => "Lighten",
            BlendMode::Screen => "Screen",
            BlendMode::ColorDodge => "Color Dodge",
            BlendMode::Add => "Add",
            BlendMode::Overlay => "Overlay",
            BlendMode::Difference => "Difference",
            BlendMode::Exclusion => "Exclusion",
            BlendMode::Subtract => "Subtract",
            BlendMode::EraseAlpha => "Erase Alpha",
            BlendMode::AddAlpha => "Add Alpha",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            BlendMode::Mix => "MIX",
            BlendMode::Darken => "DARKEN",
            BlendMode::Multiply => "MUL",
            BlendMode::ColorBurn => "COLORBURN",
            BlendMode::Lighten => "LIGHTEN",
            BlendMode::Screen => "SCREEN",
            BlendMode::ColorDodge => "COLORDODGE",
            BlendMode::Add => "ADD",
            BlendMode::Overlay => "OVERLAY",
            BlendMode::Difference => "DIFFERENCE",
            BlendMode::Exclusion => "EXCLUSION",
            BlendMode::Subtract => "SUB",
            BlendMode::EraseAlpha => "ERASE_ALPHA",
            BlendMode::AddAlpha => "ADD_ALPHA",
        }
    }
    pub fn from_id(s: &str) -> Option<BlendMode> {
        BlendMode::ALL.into_iter().find(|b| b.id() == s || b.label() == s)
    }
}

/// Blend `src` into `dst` with factor `a` (0..1). Colors are linear.
pub fn blend(mode: BlendMode, dst: [f32; 4], src: [f32; 3], a: f32) -> [f32; 4] {
    let a = a.clamp(0.0, 1.0);
    let ch = |k: usize| -> f32 {
        let (d, s) = (dst[k], src[k]);
        let f = match mode {
            BlendMode::Mix => s,
            BlendMode::Darken => d.min(s),
            BlendMode::Multiply => d * s,
            BlendMode::ColorBurn => {
                if s <= 0.0 {
                    0.0
                } else {
                    (1.0 - (1.0 - d) / s).max(0.0)
                }
            }
            BlendMode::Lighten => d.max(s),
            BlendMode::Screen => 1.0 - (1.0 - d) * (1.0 - s),
            BlendMode::ColorDodge => {
                if s >= 1.0 {
                    1.0
                } else {
                    (d / (1.0 - s)).min(1.0)
                }
            }
            BlendMode::Add => d + s,
            BlendMode::Overlay => {
                if d < 0.5 {
                    2.0 * d * s
                } else {
                    1.0 - 2.0 * (1.0 - d) * (1.0 - s)
                }
            }
            BlendMode::Difference => (d - s).abs(),
            BlendMode::Exclusion => d + s - 2.0 * d * s,
            BlendMode::Subtract => (d - s).max(0.0),
            BlendMode::EraseAlpha | BlendMode::AddAlpha => d,
        };
        d + (f - d) * a
    };
    let alpha = match mode {
        BlendMode::EraseAlpha => (dst[3] - a).max(0.0),
        BlendMode::AddAlpha => (dst[3] + a).min(1.0),
        _ => dst[3] + (1.0 - dst[3]) * a,
    };
    [ch(0), ch(1), ch(2), alpha]
}

/// Lerp of two RGBA colors.
pub fn mix4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// Linear to sRGB for one channel (color pickers show sRGB).
pub fn to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

pub fn from_srgb(c: f32) -> f32 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod blend_tests {
    use super::*;

    #[test]
    fn blend_modes_match_their_formulas() {
        let d = [0.5, 0.25, 1.0, 1.0];
        let s = [0.5, 0.5, 0.0];
        let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(blend(BlendMode::Mix, d, s, 1.0), [0.5, 0.5, 0.0, 1.0]));
        assert!(close(blend(BlendMode::Mix, d, s, 0.5), [0.5, 0.375, 0.5, 1.0]));
        assert!(close(blend(BlendMode::Add, d, s, 1.0), [1.0, 0.75, 1.0, 1.0]));
        assert!(close(blend(BlendMode::Subtract, d, s, 1.0), [0.0, 0.0, 1.0, 1.0]));
        assert!(close(blend(BlendMode::Multiply, d, s, 1.0), [0.25, 0.125, 0.0, 1.0]));
        assert!(close(blend(BlendMode::Lighten, d, s, 1.0), [0.5, 0.5, 1.0, 1.0]));
        assert!(close(blend(BlendMode::Darken, d, s, 1.0), [0.5, 0.25, 0.0, 1.0]));
        assert!(close(blend(BlendMode::Screen, d, s, 1.0), [0.75, 0.625, 1.0, 1.0]));
        assert!(close(blend(BlendMode::Difference, d, s, 1.0), [0.0, 0.25, 1.0, 1.0]));
        assert!(close(blend(BlendMode::EraseAlpha, d, s, 0.25), [0.5, 0.25, 1.0, 0.75]));
        assert!(close(blend(BlendMode::AddAlpha, [0.1, 0.1, 0.1, 0.5], s, 0.25), [0.1, 0.1, 0.1, 0.75]));
        assert!(close(blend(BlendMode::Mix, d, s, 0.0), d));
        for m in BlendMode::ALL {
            assert_eq!(BlendMode::from_id(m.id()), Some(m));
        }
    }
}
