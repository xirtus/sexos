//! Wide-gamut color spaces, ITU-R / SMPTE / IEC transfer functions,
//! 3x3 CIE gamut conversion matrices, Oklab, and HDR tone mapping.
//!
//! Linear-light spaces ([`ColorSpace::LinearRec709`] and [`ColorSpace::LinearRec2020`])
//! are referenced to [`REF_WHITE_NITS`] (`100.0` cd/m^2 = `1.0`), allowing SDR (`[0.0, 1.0]`)
//! and HDR (`> 1.0`, e.g. `10.0` = 1000 nits) to coexist in wide-gamut compositing without
//! BT.709 gamut clamping.

use libm::{cbrtf, copysignf, expf, fabsf, log10f, logf, powf};

/// Reference SDR diffuse white luminance in cd/m^2 (nits), corresponding to linear `1.0`.
pub const REF_WHITE_NITS: f32 = 100.0;

/// SMPTE ST.2084 (PQ) absolute peak luminance in cd/m^2 (nits).
pub const PQ_PEAK_NITS: f32 = 10_000.0;

/// ITU-R BT.2100 HLG nominal display peak luminance in cd/m^2 (nits).
pub const HLG_NOMINAL_PEAK_NITS: f32 = 1_000.0;

/// ITU-R BT.2408 standard SDR graphics reference white on HDR canvases (nits).
pub const BT2408_SDR_WHITE_NITS: f32 = 203.0;

/// Color gamut primaries (all sharing CIE D65 white point `(0.3127, 0.3290)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorPrimaries {
    /// ITU-R BT.709 / sRGB primaries.
    Rec709,
    /// ITU-R BT.2020 wide-gamut primaries.
    Rec2020,
    /// ITU-R BT.601 (SMPTE-C 525-line) primaries.
    Rec601,
}

impl ColorPrimaries {
    /// Alias for [`ColorPrimaries::Rec709`] (sRGB shares BT.709 primaries).
    pub const SRGB: Self = Self::Rec709;

    /// Returns the `[Kr, Kg, Kb]` Y'CbCr luma coefficients for these primaries.
    #[inline]
    pub const fn luma_weights(self) -> [f32; 3] {
        match self {
            Self::Rec709 => [0.2126, 0.7152, 0.0722],
            Self::Rec2020 => [0.2627, 0.6780, 0.0593],
            Self::Rec601 => [0.2990, 0.5870, 0.1140],
        }
    }
}

/// Supported color spaces and transfer encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// Standard sRGB (IEC 61966-2-1 transfer + BT.709 primaries).
    Srgb,
    /// ITU-R BT.709 OETF + BT.709 primaries.
    Rec709,
    /// ITU-R BT.601 OETF + BT.601 SMPTE-C primaries.
    Rec601,
    /// ITU-R BT.2020 12-bit OETF + BT.2020 primaries.
    Rec2020,
    /// Linear-light RGB with BT.709 / sRGB primaries (`1.0` = 100 nits).
    LinearRec709,
    /// Linear-light wide-gamut RGB with BT.2020 primaries (`1.0` = 100 nits).
    LinearRec2020,
    /// HDR10 SMPTE ST.2084 (PQ) perceptual quantizer + BT.2020 primaries.
    Hdr10Pq,
    /// Hybrid Log-Gamma (ARIB STD-B67 / BT.2100) + BT.2020 primaries.
    Hlg,
    /// Perceptual Oklab color space (`[L, a, b]`).
    Oklab,
}

impl ColorSpace {
    /// Returns the color primaries for RGB spaces, or `None` for perceptual [`ColorSpace::Oklab`].
    #[inline]
    pub const fn primaries(self) -> Option<ColorPrimaries> {
        match self {
            Self::Srgb | Self::Rec709 | Self::LinearRec709 => Some(ColorPrimaries::Rec709),
            Self::Rec601 => Some(ColorPrimaries::Rec601),
            Self::Rec2020 | Self::LinearRec2020 | Self::Hdr10Pq | Self::Hlg => {
                Some(ColorPrimaries::Rec2020)
            }
            Self::Oklab => None,
        }
    }

    /// Returns `true` if this color space stores linear-light RGB values.
    #[inline]
    pub const fn is_linear(self) -> bool {
        matches!(self, Self::LinearRec709 | Self::LinearRec2020)
    }

    /// Returns `true` if this color space is an HDR transfer space (`Hdr10Pq` or `Hlg`).
    #[inline]
    pub const fn is_hdr(self) -> bool {
        matches!(self, Self::Hdr10Pq | Self::Hlg)
    }

    /// Applies the 1D transfer function (EOTF / inverse OETF) to convert an encoded
    /// signal sample into linear light (`1.0 = 100 nits` for PQ).
    ///
    /// For [`ColorSpace::Hlg`], applies the 1D ARIB STD-B67 inverse OETF (`[0, 1] -> [0, 1]`);
    /// for full BT.2100 display luminance with OOTF, use [`hlg_to_linear_nits`] or [`convert_rgb`].
    #[inline]
    pub fn to_linear_scalar(self, v: f32) -> f32 {
        match self {
            Self::Srgb => srgb_to_linear(v),
            Self::Rec709 => rec709_to_linear(v),
            Self::Rec601 => rec601_to_linear(v),
            Self::Rec2020 => rec2020_to_linear(v),
            Self::LinearRec709 | Self::LinearRec2020 | Self::Oklab => v,
            Self::Hdr10Pq => pq_eotf_nits(v) / REF_WHITE_NITS,
            Self::Hlg => hlg_inverse_oetf(v),
        }
    }

    /// Applies the 1D OETF / inverse EOTF to convert linear light into an encoded signal.
    #[inline]
    pub fn from_linear_scalar(self, v: f32) -> f32 {
        match self {
            Self::Srgb => linear_to_srgb(v),
            Self::Rec709 => linear_to_rec709(v),
            Self::Rec601 => linear_to_rec601(v),
            Self::Rec2020 => linear_to_rec2020(v),
            Self::LinearRec709 | Self::LinearRec2020 | Self::Oklab => v,
            Self::Hdr10Pq => pq_oetf_nits(v * REF_WHITE_NITS),
            Self::Hlg => hlg_oetf(v),
        }
    }
}

// ---------------------------------------------------------------------------
// 1D Transfer Functions (EOTF & OETF)
// ---------------------------------------------------------------------------

/// IEC 61966-2-1 sRGB EOTF (sign-preserving for extended wide-gamut values).
#[inline]
pub fn srgb_to_linear(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = fabsf(v);
    let lin = if a <= 0.04045 {
        a / 12.92
    } else {
        powf((a + 0.055) / 1.055, 2.4)
    };
    copysignf(lin, v)
}

/// IEC 61966-2-1 sRGB OETF (sign-preserving for extended wide-gamut values).
#[inline]
pub fn linear_to_srgb(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = fabsf(v);
    let enc = if a <= 0.003_130_8 {
        a * 12.92
    } else {
        1.055 * powf(a, 1.0 / 2.4) - 0.055
    };
    copysignf(enc, v)
}

/// ITU-R BT.709 inverse OETF (sign-preserving).
#[inline]
pub fn rec709_to_linear(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = fabsf(v);
    let lin = if a < 0.081 {
        a / 4.5
    } else {
        powf((a + 0.099) / 1.099, 1.0 / 0.45)
    };
    copysignf(lin, v)
}

/// ITU-R BT.709 OETF (sign-preserving).
#[inline]
pub fn linear_to_rec709(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = fabsf(v);
    let enc = if a < 0.018 {
        4.5 * a
    } else {
        1.099 * powf(a, 0.45) - 0.099
    };
    copysignf(enc, v)
}

/// ITU-R BT.601 inverse OETF (identical curve to BT.709).
#[inline]
pub fn rec601_to_linear(v: f32) -> f32 {
    rec709_to_linear(v)
}

/// ITU-R BT.601 OETF (identical curve to BT.709).
#[inline]
pub fn linear_to_rec601(v: f32) -> f32 {
    linear_to_rec709(v)
}

/// ITU-R BT.2020 12-bit continuous constants.
const BT2020_ALPHA: f32 = 1.099_296_8;
const BT2020_BETA: f32 = 0.018_053_97;
const BT2020_CUTOFF: f32 = 4.5 * BT2020_BETA;

/// ITU-R BT.2020 inverse OETF (sign-preserving).
#[inline]
pub fn rec2020_to_linear(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = fabsf(v);
    let lin = if a < BT2020_CUTOFF {
        a / 4.5
    } else {
        powf((a + (BT2020_ALPHA - 1.0)) / BT2020_ALPHA, 1.0 / 0.45)
    };
    copysignf(lin, v)
}

/// ITU-R BT.2020 OETF (sign-preserving).
#[inline]
pub fn linear_to_rec2020(v: f32) -> f32 {
    if !v.is_finite() {
        return 0.0;
    }
    let a = fabsf(v);
    let enc = if a < BT2020_BETA {
        4.5 * a
    } else {
        BT2020_ALPHA * powf(a, 0.45) - (BT2020_ALPHA - 1.0)
    };
    copysignf(enc, v)
}

// SMPTE ST.2084 (PQ) constants in f64 for exact EOTF/OETF roundtrip precision.
const PQ_M1: f64 = 2610.0 / 16384.0;
const PQ_M2: f64 = 2523.0 / 4096.0 * 128.0;
const PQ_C1: f64 = 3424.0 / 4096.0;
const PQ_C2: f64 = 2413.0 / 4096.0 * 32.0;
const PQ_C3: f64 = 2392.0 / 4096.0 * 32.0;

/// SMPTE ST.2084 PQ EOTF: maps `[0.0, 1.0]` PQ signal to normalized linear `[0.0, 1.0]` (`1.0` = 10,000 nits).
#[inline]
pub fn pq_eotf(e_prime: f32) -> f32 {
    if !e_prime.is_finite() || e_prime <= 0.0 {
        return 0.0;
    }
    let ep = (e_prime as f64).min(1.0);
    let p = libm::pow(ep, 1.0 / PQ_M2);
    let num = (p - PQ_C1).max(0.0);
    let den = (PQ_C2 - PQ_C3 * p).max(1e-15);
    libm::pow(num / den, 1.0 / PQ_M1) as f32
}

/// SMPTE ST.2084 PQ inverse EOTF (OETF): maps normalized linear `[0.0, 1.0]` (`1.0` = 10,000 nits) to `[0.0, 1.0]`.
#[inline]
pub fn pq_oetf(y: f32) -> f32 {
    if !y.is_finite() || y <= 0.0 {
        return 0.0;
    }
    let y_clamped = (y as f64).min(1.0);
    let ym = libm::pow(y_clamped, PQ_M1);
    let num = PQ_C1 + PQ_C2 * ym;
    let den = 1.0 + PQ_C3 * ym;
    libm::pow((num / den).max(0.0), PQ_M2) as f32
}

/// SMPTE ST.2084 PQ EOTF returning absolute luminance in cd/m^2 (nits, `[0.0, 10000.0]`).
#[inline]
pub fn pq_eotf_nits(e_prime: f32) -> f32 {
    pq_eotf(e_prime) * PQ_PEAK_NITS
}

/// SMPTE ST.2084 PQ inverse EOTF taking absolute luminance in cd/m^2 (nits, `[0.0, 10000.0]`).
#[inline]
pub fn pq_oetf_nits(nits: f32) -> f32 {
    pq_oetf(nits / PQ_PEAK_NITS)
}

// ARIB STD-B67 (HLG) constants.
const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 1.0 - 4.0 * HLG_A; // 0.28466892
const HLG_C: f32 = 0.559_910_73; // 0.5 - HLG_A * ln(4 * HLG_A)

/// ARIB STD-B67 HLG OETF: maps scene-linear `[0.0, 1.0]` to HLG signal `[0.0, 1.0]`.
#[inline]
pub fn hlg_oetf(e: f32) -> f32 {
    if !e.is_finite() || e <= 0.0 {
        return 0.0;
    }
    if e <= 1.0 / 12.0 {
        libm::sqrtf(3.0 * e)
    } else {
        HLG_A * logf((12.0 * e - HLG_B).max(1e-12)) + HLG_C
    }
}

/// ARIB STD-B67 HLG inverse OETF: maps HLG signal `[0.0, 1.0]` to scene-linear `[0.0, 1.0]`.
#[inline]
pub fn hlg_inverse_oetf(ep: f32) -> f32 {
    if !ep.is_finite() || ep <= 0.0 {
        return 0.0;
    }
    if ep <= 0.5 {
        (ep * ep) / 3.0
    } else {
        (expf((ep - HLG_C) / HLG_A) + HLG_B) / 12.0
    }
}

/// Alias for [`hlg_inverse_oetf`].
#[inline]
pub fn hlg_eotf(ep: f32) -> f32 {
    hlg_inverse_oetf(ep)
}

/// Computes the ITU-R BT.2100 HLG system gamma for a given display peak luminance (nits).
#[inline]
pub fn hlg_system_gamma(peak_nits: f32) -> f32 {
    let peak = if peak_nits.is_finite() && peak_nits > 1.0 {
        peak_nits
    } else {
        HLG_NOMINAL_PEAK_NITS
    };
    (1.2 + 0.42 * log10f(peak / HLG_NOMINAL_PEAK_NITS)).max(0.5)
}

/// Converts HLG signal RGB `[0.0, 1.0]` to display-linear RGB in nits using the BT.2100 OOTF.
pub fn hlg_to_linear_nits(rgb: [f32; 3], peak_nits: f32) -> [f32; 3] {
    let e = [
        hlg_inverse_oetf(rgb[0]),
        hlg_inverse_oetf(rgb[1]),
        hlg_inverse_oetf(rgb[2]),
    ];
    let ys = 0.2627 * e[0] + 0.6780 * e[1] + 0.0593 * e[2];
    if ys <= 0.0 || !ys.is_finite() {
        return [0.0, 0.0, 0.0];
    }
    let gamma = hlg_system_gamma(peak_nits);
    let scale = peak_nits * powf(ys, gamma - 1.0);
    [e[0] * scale, e[1] * scale, e[2] * scale]
}

/// Converts display-linear RGB in nits to HLG signal RGB `[0.0, 1.0]` using the inverse BT.2100 OOTF.
pub fn linear_nits_to_hlg(rgb_nits: [f32; 3], peak_nits: f32) -> [f32; 3] {
    let l = [
        if rgb_nits[0].is_finite() { rgb_nits[0].max(0.0) } else { 0.0 },
        if rgb_nits[1].is_finite() { rgb_nits[1].max(0.0) } else { 0.0 },
        if rgb_nits[2].is_finite() { rgb_nits[2].max(0.0) } else { 0.0 },
    ];
    let yd = 0.2627 * l[0] + 0.6780 * l[1] + 0.0593 * l[2];
    if yd <= 0.0 || peak_nits <= 0.0 {
        return [0.0, 0.0, 0.0];
    }
    let gamma = hlg_system_gamma(peak_nits);
    let inv_scale = (1.0 / peak_nits) * powf(yd / peak_nits, (1.0 - gamma) / gamma);
    [
        hlg_oetf(l[0] * inv_scale),
        hlg_oetf(l[1] * inv_scale),
        hlg_oetf(l[2] * inv_scale),
    ]
}

// ---------------------------------------------------------------------------
// 3x3 CIE D65 Gamut Matrices & Oklab
// ---------------------------------------------------------------------------

/// Multiplies a 3x3 row-major matrix by a 3-element column vector.
#[inline]
pub fn mul_mat3(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// Linear Rec.709 / sRGB -> CIE XYZ (D65 `(0.3127, 0.3290)`).
pub const REC709_TO_XYZ: [[f32; 3]; 3] = [
    [0.412_390_8, 0.357_584_34, 0.180_480_79],
    [0.212_639_01, 0.715_168_7, 0.072_192_32],
    [0.019_330_82, 0.119_194_78, 0.950_532_14],
];

/// CIE XYZ (D65) -> Linear Rec.709 / sRGB.
pub const XYZ_TO_REC709: [[f32; 3]; 3] = [
    [3.240_97, -1.537_383_2, -0.498_610_76],
    [-0.969_243_65, 1.875_967_5, 0.041_555_06],
    [0.055_630_08, -0.203_976_96, 1.056_971_5],
];

/// Linear Rec.2020 -> CIE XYZ (D65 `(0.3127, 0.3290)`).
pub const REC2020_TO_XYZ: [[f32; 3]; 3] = [
    [0.636_958_06, 0.144_616_9, 0.168_880_98],
    [0.262_700_2, 0.677_998_07, 0.059_301_72],
    [0.0, 0.028_072_69, 1.060_985_1],
];

/// CIE XYZ (D65) -> Linear Rec.2020.
pub const XYZ_TO_REC2020: [[f32; 3]; 3] = [
    [1.716_651_2, -0.355_670_78, -0.253_366_29],
    [-0.666_684_33, 1.616_481_2, 0.015_768_55],
    [0.017_639_86, -0.042_770_61, 0.942_103_15],
];

/// Linear Rec.601 (SMPTE-C) -> CIE XYZ (D65).
pub const REC601_TO_XYZ: [[f32; 3]; 3] = [
    [0.393_520_9, 0.365_258_07, 0.191_676_95],
    [0.212_376_36, 0.701_059_9, 0.086_563_78],
    [0.018_739_09, 0.111_933_93, 0.958_384_75],
];

/// CIE XYZ (D65) -> Linear Rec.601 (SMPTE-C).
pub const XYZ_TO_REC601: [[f32; 3]; 3] = [
    [3.506_003_4, -1.739_790_7, -0.544_058_26],
    [-1.069_047_6, 1.977_778_9, 0.035_171_42],
    [0.056_306_59, -0.196_975_65, 1.049_952_3],
];

/// Linear Rec.709 -> Linear Rec.2020 (ITU-R BT.2087).
pub const REC709_TO_REC2020: [[f32; 3]; 3] = [
    [0.627_403_9, 0.329_283_03, 0.043_313_07],
    [0.069_097_29, 0.919_540_4, 0.011_362_32],
    [0.016_391_44, 0.088_013_31, 0.895_595_25],
];

/// Linear Rec.2020 -> Linear Rec.709 (ITU-R BT.2087).
pub const REC2020_TO_REC709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_86],
    [-0.124_550_47, 1.132_899_9, -0.008_349_42],
    [-0.018_150_76, -0.100_578_9, 1.118_729_7],
];

/// Linear Rec.601 (SMPTE-C) -> Linear Rec.709.
pub const REC601_TO_REC709: [[f32; 3]; 3] = [
    [0.939_542_06, 0.050_181_36, 0.010_276_58],
    [0.017_772_22, 0.965_792_84, 0.016_434_91],
    [-0.001_621_6, -0.004_369_75, 1.005_991_3],
];

/// Linear Rec.709 -> Linear Rec.601 (SMPTE-C).
pub const REC709_TO_REC601: [[f32; 3]; 3] = [
    [1.065_379, -0.055_400_87, -0.009_978_16],
    [-0.019_632_55, 1.036_363_1, -0.016_730_54],
    [0.001_632_05, 0.004_412_37, 0.993_955_6],
];

/// Linear Rec.601 (SMPTE-C) -> Linear Rec.2020.
pub const REC601_TO_REC2020: [[f32; 3]; 3] = [
    [0.595_254_2, 0.349_313_9, 0.055_431_87],
    [0.081_243_66, 0.891_503_3, 0.027_253_04],
    [0.015_512_34, 0.081_911_64, 0.902_576],
];

/// Linear Rec.2020 -> Linear Rec.601 (SMPTE-C).
pub const REC2020_TO_REC601: [[f32; 3]; 3] = [
    [1.776_133_7, -0.687_820_6, -0.088_313_02],
    [-0.161_375_52, 1.187_315_3, -0.025_939_76],
    [-0.015_880_61, -0.095_931_24, 1.111_811_8],
];

/// Returns the 3x3 linear conversion matrix between two [`ColorPrimaries`].
#[inline]
pub const fn primaries_matrix(from: ColorPrimaries, to: ColorPrimaries) -> [[f32; 3]; 3] {
    match (from, to) {
        (ColorPrimaries::Rec709, ColorPrimaries::Rec709)
        | (ColorPrimaries::Rec2020, ColorPrimaries::Rec2020)
        | (ColorPrimaries::Rec601, ColorPrimaries::Rec601) => {
            [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        }
        (ColorPrimaries::Rec709, ColorPrimaries::Rec2020) => REC709_TO_REC2020,
        (ColorPrimaries::Rec2020, ColorPrimaries::Rec709) => REC2020_TO_REC709,
        (ColorPrimaries::Rec601, ColorPrimaries::Rec709) => REC601_TO_REC709,
        (ColorPrimaries::Rec709, ColorPrimaries::Rec601) => REC709_TO_REC601,
        (ColorPrimaries::Rec601, ColorPrimaries::Rec2020) => REC601_TO_REC2020,
        (ColorPrimaries::Rec2020, ColorPrimaries::Rec601) => REC2020_TO_REC601,
    }
}

/// Converts linear RGB between two [`ColorPrimaries`] without gamut clipping.
#[inline]
pub fn convert_primaries(rgb: [f32; 3], from: ColorPrimaries, to: ColorPrimaries) -> [f32; 3] {
    if from == to {
        return rgb;
    }
    let m = primaries_matrix(from, to);
    mul_mat3(&m, rgb)
}

/// Converts linear Rec.709 RGB to CIE XYZ (D65).
#[inline]
pub fn linear_rec709_to_xyz(rgb: [f32; 3]) -> [f32; 3] {
    mul_mat3(&REC709_TO_XYZ, rgb)
}

/// Converts CIE XYZ (D65) to linear Rec.709 RGB.
#[inline]
pub fn xyz_to_linear_rec709(xyz: [f32; 3]) -> [f32; 3] {
    mul_mat3(&XYZ_TO_REC709, xyz)
}

/// Converts linear Rec.2020 RGB to CIE XYZ (D65).
#[inline]
pub fn linear_rec2020_to_xyz(rgb: [f32; 3]) -> [f32; 3] {
    mul_mat3(&REC2020_TO_XYZ, rgb)
}

/// Converts CIE XYZ (D65) to linear Rec.2020 RGB.
#[inline]
pub fn xyz_to_linear_rec2020(xyz: [f32; 3]) -> [f32; 3] {
    mul_mat3(&XYZ_TO_REC2020, xyz)
}

/// Converts linear Rec.709 RGB to linear Rec.2020 RGB.
#[inline]
pub fn linear_rec709_to_rec2020(rgb: [f32; 3]) -> [f32; 3] {
    mul_mat3(&REC709_TO_REC2020, rgb)
}

/// Converts linear Rec.2020 RGB to linear Rec.709 RGB (unclamped).
#[inline]
pub fn linear_rec2020_to_rec709(rgb: [f32; 3]) -> [f32; 3] {
    mul_mat3(&REC2020_TO_REC709, rgb)
}

// Oklab matrices (Bjorn Ottosson 2020).
const OKLAB_M1: [[f32; 3]; 3] = [
    [0.412_221_46, 0.536_332_55, 0.051_445_99],
    [0.211_903_5, 0.680_699_5, 0.107_396_96],
    [0.088_302_46, 0.281_718_85, 0.629_978_7],
];

const OKLAB_M2: [[f32; 3]; 3] = [
    [0.210_454_26, 0.793_617_8, -0.004_072_05],
    [1.977_998_5, -2.428_592_2, 0.450_593_7],
    [0.025_904_04, 0.782_771_77, -0.808_675_77],
];

const OKLAB_M1_INV: [[f32; 3]; 3] = [
    [4.076_741_7, -3.307_711_6, 0.230_969_94],
    [-1.268_438, 2.609_757_4, -0.341_319_4],
    [-0.004_196_09, -0.703_418_6, 1.707_614_7],
];

const OKLAB_M2_INV: [[f32; 3]; 3] = [
    [1.0, 0.396_337_78, 0.215_803_76],
    [1.0, -0.105_561_34, -0.063_854_17],
    [1.0, -0.089_484_18, -1.291_485_5],
];

/// Converts linear Rec.709 RGB to perceptual Oklab `[L, a, b]`.
#[inline]
pub fn linear_rec709_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    let lms = mul_mat3(&OKLAB_M1, rgb);
    let lms_cbrt = [cbrtf(lms[0]), cbrtf(lms[1]), cbrtf(lms[2])];
    mul_mat3(&OKLAB_M2, lms_cbrt)
}

/// Converts perceptual Oklab `[L, a, b]` to linear Rec.709 RGB.
#[inline]
pub fn oklab_to_linear_rec709(lab: [f32; 3]) -> [f32; 3] {
    let lms_cbrt = mul_mat3(&OKLAB_M2_INV, lab);
    let lms = [
        lms_cbrt[0] * lms_cbrt[0] * lms_cbrt[0],
        lms_cbrt[1] * lms_cbrt[1] * lms_cbrt[1],
        lms_cbrt[2] * lms_cbrt[2] * lms_cbrt[2],
    ];
    mul_mat3(&OKLAB_M1_INV, lms)
}

/// Converts linear Rec.2020 RGB to perceptual Oklab `[L, a, b]` without gamut clipping.
#[inline]
pub fn linear_rec2020_to_oklab(rgb: [f32; 3]) -> [f32; 3] {
    linear_rec709_to_oklab(linear_rec2020_to_rec709(rgb))
}

/// Converts perceptual Oklab `[L, a, b]` to linear Rec.2020 RGB.
#[inline]
pub fn oklab_to_linear_rec2020(lab: [f32; 3]) -> [f32; 3] {
    linear_rec709_to_rec2020(oklab_to_linear_rec709(lab))
}

/// Converts a 3-channel color triplet between any two [`ColorSpace`]s without gamut clamping.
///
/// Transformations between spaces sharing Rec.2020 primaries stay directly in [`ColorSpace::LinearRec2020`],
/// avoiding BT.709 gamut compression or clamping.
pub fn convert_rgb(rgb: [f32; 3], from: ColorSpace, to: ColorSpace) -> [f32; 3] {
    if from == to {
        return rgb;
    }

    // Fast path: keep Rec.2020-family conversions directly in LinearRec2020.
    let from_2020 = from.primaries() == Some(ColorPrimaries::Rec2020);
    let to_2020 = to.primaries() == Some(ColorPrimaries::Rec2020);
    if from_2020 && to_2020 {
        let lin2020 = to_linear_rec2020_direct(rgb, from);
        return from_linear_rec2020_direct(lin2020, to);
    }

    // Fast path: keep Rec.709-family conversions directly in LinearRec709.
    let from_709 = from.primaries() == Some(ColorPrimaries::Rec709);
    let to_709 = to.primaries() == Some(ColorPrimaries::Rec709);
    if from_709 && to_709 {
        let lin709 = [
            from.to_linear_scalar(rgb[0]),
            from.to_linear_scalar(rgb[1]),
            from.to_linear_scalar(rgb[2]),
        ];
        return [
            to.from_linear_scalar(lin709[0]),
            to.from_linear_scalar(lin709[1]),
            to.from_linear_scalar(lin709[2]),
        ];
    }

    // General path via LinearRec709 or LinearRec2020.
    if to == ColorSpace::Oklab {
        if from_2020 {
            return linear_rec2020_to_oklab(to_linear_rec2020_direct(rgb, from));
        }
        let lin709 = to_linear_rec709(rgb, from);
        return linear_rec709_to_oklab(lin709);
    }
    if from == ColorSpace::Oklab {
        if to_2020 {
            return from_linear_rec2020_direct(oklab_to_linear_rec2020(rgb), to);
        }
        let lin709 = oklab_to_linear_rec709(rgb);
        return from_linear_rec709(lin709, to);
    }

    if to_2020 {
        let lin709 = to_linear_rec709(rgb, from);
        let lin2020 = linear_rec709_to_rec2020(lin709);
        return from_linear_rec2020_direct(lin2020, to);
    }

    let lin709 = to_linear_rec709(rgb, from);
    from_linear_rec709(lin709, to)
}

fn to_linear_rec2020_direct(rgb: [f32; 3], space: ColorSpace) -> [f32; 3] {
    match space {
        ColorSpace::LinearRec2020 => rgb,
        ColorSpace::Rec2020 => [
            rec2020_to_linear(rgb[0]),
            rec2020_to_linear(rgb[1]),
            rec2020_to_linear(rgb[2]),
        ],
        ColorSpace::Hdr10Pq => [
            pq_eotf_nits(rgb[0]) / REF_WHITE_NITS,
            pq_eotf_nits(rgb[1]) / REF_WHITE_NITS,
            pq_eotf_nits(rgb[2]) / REF_WHITE_NITS,
        ],
        ColorSpace::Hlg => {
            let nits = hlg_to_linear_nits(rgb, HLG_NOMINAL_PEAK_NITS);
            [
                nits[0] / REF_WHITE_NITS,
                nits[1] / REF_WHITE_NITS,
                nits[2] / REF_WHITE_NITS,
            ]
        }
        _ => convert_primaries(
            to_linear_rec709(rgb, space),
            ColorPrimaries::Rec709,
            ColorPrimaries::Rec2020,
        ),
    }
}

fn from_linear_rec2020_direct(lin: [f32; 3], space: ColorSpace) -> [f32; 3] {
    match space {
        ColorSpace::LinearRec2020 => lin,
        ColorSpace::Rec2020 => [
            linear_to_rec2020(lin[0]),
            linear_to_rec2020(lin[1]),
            linear_to_rec2020(lin[2]),
        ],
        ColorSpace::Hdr10Pq => [
            pq_oetf_nits(lin[0].max(0.0) * REF_WHITE_NITS),
            pq_oetf_nits(lin[1].max(0.0) * REF_WHITE_NITS),
            pq_oetf_nits(lin[2].max(0.0) * REF_WHITE_NITS),
        ],
        ColorSpace::Hlg => linear_nits_to_hlg(
            [
                lin[0].max(0.0) * REF_WHITE_NITS,
                lin[1].max(0.0) * REF_WHITE_NITS,
                lin[2].max(0.0) * REF_WHITE_NITS,
            ],
            HLG_NOMINAL_PEAK_NITS,
        ),
        _ => from_linear_rec709(linear_rec2020_to_rec709(lin), space),
    }
}

fn to_linear_rec709(rgb: [f32; 3], space: ColorSpace) -> [f32; 3] {
    match space {
        ColorSpace::Srgb => [
            srgb_to_linear(rgb[0]),
            srgb_to_linear(rgb[1]),
            srgb_to_linear(rgb[2]),
        ],
        ColorSpace::Rec709 => [
            rec709_to_linear(rgb[0]),
            rec709_to_linear(rgb[1]),
            rec709_to_linear(rgb[2]),
        ],
        ColorSpace::LinearRec709 => rgb,
        ColorSpace::Rec601 => {
            let lin601 = [
                rec601_to_linear(rgb[0]),
                rec601_to_linear(rgb[1]),
                rec601_to_linear(rgb[2]),
            ];
            mul_mat3(&REC601_TO_REC709, lin601)
        }
        ColorSpace::Rec2020 | ColorSpace::LinearRec2020 | ColorSpace::Hdr10Pq | ColorSpace::Hlg => {
            linear_rec2020_to_rec709(to_linear_rec2020_direct(rgb, space))
        }
        ColorSpace::Oklab => oklab_to_linear_rec709(rgb),
    }
}

fn from_linear_rec709(lin709: [f32; 3], space: ColorSpace) -> [f32; 3] {
    match space {
        ColorSpace::Srgb => [
            linear_to_srgb(lin709[0]),
            linear_to_srgb(lin709[1]),
            linear_to_srgb(lin709[2]),
        ],
        ColorSpace::Rec709 => [
            linear_to_rec709(lin709[0]),
            linear_to_rec709(lin709[1]),
            linear_to_rec709(lin709[2]),
        ],
        ColorSpace::LinearRec709 => lin709,
        ColorSpace::Rec601 => {
            let lin601 = mul_mat3(&REC709_TO_REC601, lin709);
            [
                linear_to_rec601(lin601[0]),
                linear_to_rec601(lin601[1]),
                linear_to_rec601(lin601[2]),
            ]
        }
        ColorSpace::Rec2020 | ColorSpace::LinearRec2020 | ColorSpace::Hdr10Pq | ColorSpace::Hlg => {
            from_linear_rec2020_direct(linear_rec709_to_rec2020(lin709), space)
        }
        ColorSpace::Oklab => linear_rec709_to_oklab(lin709),
    }
}

// ---------------------------------------------------------------------------
// HDR Tone Mapping & SDR-on-HDR Scaling
// ---------------------------------------------------------------------------

/// HDR tone mapping curve algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToneMapMode {
    /// ITU-R BT.2390 EETF Hermite spline compression in PQ perceptual space.
    Bt2390,
    /// Extended Reinhard luminance-mapped compression with configurable white point.
    Reinhard,
    /// John Hable (Uncharted 2) filmic S-curve normalized to target peak.
    HableFilmic,
    /// Hard clip to `[0.0, target_peak_nits / 100.0]`.
    Clip,
}

/// Configuration for HDR-to-SDR or HDR-to-HDR tone mapping.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMapConfig {
    pub mode: ToneMapMode,
    /// Mastering/source peak luminance in nits (e.g. `1000.0` or `4000.0`).
    pub source_peak_nits: f32,
    /// Target display peak luminance in nits (e.g. `100.0` for SDR, `400.0` for DisplayHDR 400).
    pub target_peak_nits: f32,
}

impl Default for ToneMapConfig {
    fn default() -> Self {
        Self {
            mode: ToneMapMode::Bt2390,
            source_peak_nits: 1_000.0,
            target_peak_nits: REF_WHITE_NITS,
        }
    }
}

/// Returns the linear multiplier for compositing SDR graphics (`[0.0, 1.0]`, where `1.0` = SDR white)
/// onto an HDR linear canvas referenced at `1.0 = 100 nits`.
///
/// For example, ITU-R BT.2408 recommends `paper_white_nits = 203.0`, returning `2.03`.
#[inline]
pub fn sdr_on_hdr_scale(paper_white_nits: f32) -> f32 {
    if !paper_white_nits.is_finite() || paper_white_nits <= 0.0 {
        return 0.0;
    }
    paper_white_nits / REF_WHITE_NITS
}

#[inline]
fn hable_partial(x: f32) -> f32 {
    const A: f32 = 0.15;
    const B: f32 = 0.50;
    const C: f32 = 0.10;
    const D: f32 = 0.20;
    const E: f32 = 0.02;
    const F: f32 = 0.30;
    ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F
}

fn bt2390_map_nits(nits: f32, source_peak_nits: f32, target_peak_nits: f32) -> f32 {
    if nits <= 0.0 || !nits.is_finite() {
        return 0.0;
    }
    if source_peak_nits <= target_peak_nits {
        return nits.min(target_peak_nits);
    }
    let lw = pq_oetf_nits(source_peak_nits).max(1e-6);
    let max_lum = (pq_oetf_nits(target_peak_nits) / lw).clamp(0.0, 1.0);
    let ks = (1.5 * max_lum - 0.5).clamp(0.0, 1.0);
    let e1 = (pq_oetf_nits(nits) / lw).clamp(0.0, 1.0);
    let e2 = if e1 < ks || ks >= 1.0 - 1e-6 {
        e1
    } else {
        let t = (e1 - ks) / (1.0 - ks);
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * ks
            + (t3 - 2.0 * t2 + t) * (1.0 - ks)
            + (-2.0 * t3 + 3.0 * t2) * max_lum
    };
    pq_eotf_nits((e2 * lw).clamp(0.0, 1.0)).min(target_peak_nits)
}

/// Applies HDR tone mapping to a linear-light RGB triplet (`1.0` = 100 nits).
///
/// Preserves chromaticity ratios (`R:G:B`) by mapping the peak channel `max(R, G, B)`
/// so all output channels remain within `[0.0, target_peak_nits / 100.0]` without hue shift.
pub fn tone_map(
    rgb: [f32; 3],
    mode: ToneMapMode,
    source_peak_nits: f32,
    target_peak_nits: f32,
) -> [f32; 3] {
    let r = if rgb[0].is_finite() { rgb[0].max(0.0) } else { 0.0 };
    let g = if rgb[1].is_finite() { rgb[1].max(0.0) } else { 0.0 };
    let b = if rgb[2].is_finite() { rgb[2].max(0.0) } else { 0.0 };

    let target_nits = if target_peak_nits.is_finite() && target_peak_nits > 0.0 {
        target_peak_nits
    } else {
        REF_WHITE_NITS
    };
    let source_nits = if source_peak_nits.is_finite() && source_peak_nits > 0.0 {
        source_peak_nits
    } else {
        target_nits
    };
    let target_lin = target_nits / REF_WHITE_NITS;
    let source_lin = source_nits / REF_WHITE_NITS;

    let max_c = r.max(g).max(b);
    if max_c <= 0.0 {
        return [0.0, 0.0, 0.0];
    }

    let mapped_max = match mode {
        ToneMapMode::Clip => max_c.min(target_lin),
        ToneMapMode::Bt2390 => {
            bt2390_map_nits(max_c * REF_WHITE_NITS, source_nits, target_nits) / REF_WHITE_NITS
        }
        ToneMapMode::Reinhard => {
            let x = (max_c / target_lin).min(source_lin / target_lin);
            let w = (source_lin / target_lin).max(1.0);
            let y = x * (1.0 + x / (w * w)) / (1.0 + x);
            (y * target_lin).min(target_lin)
        }
        ToneMapMode::HableFilmic => {
            let x = (max_c / target_lin).min(source_lin / target_lin);
            let w = (source_lin / target_lin).max(1.0);
            let denom = hable_partial(2.0 * w).max(1e-6);
            let y = hable_partial(2.0 * x) / denom;
            (y * target_lin).min(target_lin)
        }
    };

    let scale = mapped_max / max_c;
    [r * scale, g * scale, b * scale]
}

/// Applies HDR tone mapping using a [`ToneMapConfig`].
#[inline]
pub fn tone_map_rgb(rgb: [f32; 3], cfg: ToneMapConfig) -> [f32; 3] {
    tone_map(rgb, cfg.mode, cfg.source_peak_nits, cfg.target_peak_nits)
}
