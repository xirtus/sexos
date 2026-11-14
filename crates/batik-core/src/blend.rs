// SPDX-License-Identifier: GPL-3.0-only
//! Blend modes and exact straight-alpha Porter-Duff SrcOver compositing.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

impl BlendMode {
    pub fn from_psd_key(key: &[u8; 4]) -> Option<Self> {
        match key {
            b"norm" => Some(BlendMode::Normal),
            b"mul " => Some(BlendMode::Multiply),
            b"scrn" => Some(BlendMode::Screen),
            b"over" => Some(BlendMode::Overlay),
            b"dark" => Some(BlendMode::Darken),
            b"lite" => Some(BlendMode::Lighten),
            b"div " => Some(BlendMode::ColorDodge),
            b"idiv" => Some(BlendMode::ColorBurn),
            b"hLit" => Some(BlendMode::HardLight),
            b"sLit" => Some(BlendMode::SoftLight),
            b"diff" => Some(BlendMode::Difference),
            b"smud" => Some(BlendMode::Exclusion),
            _ => None,
        }
    }

    pub fn to_psd_key(&self) -> [u8; 4] {
        match self {
            BlendMode::Normal => *b"norm",
            BlendMode::Multiply => *b"mul ",
            BlendMode::Screen => *b"scrn",
            BlendMode::Overlay => *b"over",
            BlendMode::Darken => *b"dark",
            BlendMode::Lighten => *b"lite",
            BlendMode::ColorDodge => *b"div ",
            BlendMode::ColorBurn => *b"idiv",
            BlendMode::HardLight => *b"hLit",
            BlendMode::SoftLight => *b"sLit",
            BlendMode::Difference => *b"diff",
            BlendMode::Exclusion => *b"smud",
        }
    }

    pub fn from_ora_op(op: &str) -> Option<Self> {
        match op {
            "svg:src-over" => Some(BlendMode::Normal),
            "svg:multiply" => Some(BlendMode::Multiply),
            "svg:screen" => Some(BlendMode::Screen),
            "svg:overlay" => Some(BlendMode::Overlay),
            "svg:darken" => Some(BlendMode::Darken),
            "svg:lighten" => Some(BlendMode::Lighten),
            "svg:color-dodge" => Some(BlendMode::ColorDodge),
            "svg:color-burn" => Some(BlendMode::ColorBurn),
            "svg:hard-light" => Some(BlendMode::HardLight),
            "svg:soft-light" => Some(BlendMode::SoftLight),
            "svg:difference" => Some(BlendMode::Difference),
            "svg:exclusion" => Some(BlendMode::Exclusion),
            _ => None,
        }
    }

    pub fn to_ora_op(&self) -> &'static str {
        match self {
            BlendMode::Normal => "svg:src-over",
            BlendMode::Multiply => "svg:multiply",
            BlendMode::Screen => "svg:screen",
            BlendMode::Overlay => "svg:overlay",
            BlendMode::Darken => "svg:darken",
            BlendMode::Lighten => "svg:lighten",
            BlendMode::ColorDodge => "svg:color-dodge",
            BlendMode::ColorBurn => "svg:color-burn",
            BlendMode::HardLight => "svg:hard-light",
            BlendMode::SoftLight => "svg:soft-light",
            BlendMode::Difference => "svg:difference",
            BlendMode::Exclusion => "svg:exclusion",
        }
    }
}

#[inline(always)]
pub fn split_argb(c: u32) -> (u8, u8, u8, u8) {
    (
        ((c >> 24) & 0xFF) as u8,
        ((c >> 16) & 0xFF) as u8,
        ((c >> 8) & 0xFF) as u8,
        (c & 0xFF) as u8,
    )
}

#[inline(always)]
pub fn pack_argb(a: u8, r: u8, g: u8, b: u8) -> u32 {
    ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

#[inline(always)]
fn blend_channel(b: f32, s: f32, mode: BlendMode) -> f32 {
    match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => b * s,
        BlendMode::Screen => 1.0 - (1.0 - b) * (1.0 - s),
        BlendMode::Overlay => {
            if b <= 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        BlendMode::Darken => b.min(s),
        BlendMode::Lighten => b.max(s),
        BlendMode::ColorDodge => {
            if b <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (b / (1.0 - s)).min(1.0)
            }
        }
        BlendMode::ColorBurn => {
            if b >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                (1.0 - (1.0 - b) / s).max(0.0)
            }
        }
        BlendMode::HardLight => {
            if s <= 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        BlendMode::SoftLight => {
            if s <= 0.5 {
                b - (1.0 - 2.0 * s) * b * (1.0 - b)
            } else {
                let d = if b <= 0.25 {
                    ((16.0 * b - 12.0) * b + 4.0) * b
                } else {
                    libm::sqrtf(b)
                };
                b + (2.0 * s - 1.0) * (d - b)
            }
        }
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Exclusion => b + s - 2.0 * b * s,
    }
}

/// Blends a source pixel over a backdrop pixel using straight alpha Porter-Duff SrcOver.
///
/// - `backdrop`: existing pixel (`0xAARRGGBB` straight alpha)
/// - `source`: incoming layer pixel (`0xAARRGGBB` straight alpha)
/// - `mode`: layer blend mode
/// - `opacity`: layer opacity (0..=255)
/// - `mask`: layer mask value (0..=255)
pub fn blend_pixel(backdrop: u32, source: u32, mode: BlendMode, opacity: u8, mask: u8) -> u32 {
    let (sa_raw, sr_raw, sg_raw, sb_raw) = split_argb(source);
    if sa_raw == 0 || opacity == 0 || mask == 0 {
        return backdrop;
    }

    let (ba_raw, br_raw, bg_raw, bb_raw) = split_argb(backdrop);
    let sa = (sa_raw as f32 / 255.0) * (opacity as f32 / 255.0) * (mask as f32 / 255.0);
    let ba = ba_raw as f32 / 255.0;

    let ra = sa + ba * (1.0 - sa);
    if ra <= 0.0 {
        return 0;
    }

    let br = br_raw as f32 / 255.0;
    let bg = bg_raw as f32 / 255.0;
    let bb = bb_raw as f32 / 255.0;

    let sr = sr_raw as f32 / 255.0;
    let sg = sg_raw as f32 / 255.0;
    let sb = sb_raw as f32 / 255.0;

    let fr = blend_channel(br, sr, mode);
    let fg = blend_channel(bg, sg, mode);
    let fb = blend_channel(bb, sb, mode);

    let comp = |b_c: f32, s_c: f32, f_c: f32| -> u8 {
        let cr = ((1.0 - ba) * sa * s_c + (1.0 - sa) * ba * b_c + sa * ba * f_c) / ra;
        libm::roundf(cr * 255.0).clamp(0.0, 255.0) as u8
    };

    let out_r = comp(br, sr, fr);
    let out_g = comp(bg, sg, fg);
    let out_b = comp(bb, sb, fb);
    let out_a = libm::roundf(ra * 255.0).clamp(0.0, 255.0) as u8;

    pack_argb(out_a, out_r, out_g, out_b)
}
