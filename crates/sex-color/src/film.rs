//! OmaRAW-inspired physical film stock emulation curves, 3-layer dye cross-matrices,
//! halation/grain strength scalars, Kodak Cineon 10-bit Log, and 3D LUT baking.

use alloc::string::ToString;
use alloc::vec::Vec;
use libm::{expf, log10f, powf};

use crate::frame::{LinearFrame, RgbaF32};
use crate::lut::Lut3D;
use crate::space::{linear_to_srgb, mul_mat3, srgb_to_linear};

const LN_10: f32 = core::f32::consts::LN_10;
const CINEON_REF_WHITE: f32 = 685.0;
const CINEON_REF_BLACK: f32 = 95.0;
const CINEON_CODE_RANGE: f32 = 1023.0;
const CINEON_STEP: f32 = 300.0; // 0.6 display gamma / 0.002 density per code step
const CINEON_LIN_BLACK: f32 = 0.010_797_752; // 10^((95 - 685) / 300)

/// Physical cinema and photographic film stock presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilmStock {
    /// Kodak Vision 2383 cinema print stock (deep toe, teal shadows, warm highlights).
    Kodak2383Print,
    /// Kodak Vision Premier 2393 print stock (deeper Dmax, higher contrast & saturation).
    Kodak2393Print,
    /// Kodak Vision3 500T 5219 tungsten cinema negative (wide latitude, soft shoulder).
    KodakVision3_5219,
    /// Kodak Portra 160 fine-grain portrait negative (gentle contrast, warm skin tones).
    Portra160,
    /// Kodak Portra 400 portrait negative (creamy highlight rolloff, luminous skin).
    Portra400,
    /// Kodak Portra 800 high-speed portrait negative (punchier contrast, richer grain).
    Portra800,
    /// Kodak Ektar 100 vivid commercial/landscape negative (high color separation).
    Ektar100,
    /// Kodak Gold 200 vintage consumer negative (golden-amber warmth).
    Gold200,
    /// Fujifilm Eterna-CP 3510 print stock (emerald greens, smooth highlight rolloff).
    Fuji3510Print,
    /// Fujifilm Eterna-CP 3513DI print stock (crisp blacks, cool-cyan shadow separation).
    Fuji3513Di,
    /// Agfacolor CP30 European print stock (restrained greens, warm bronze skin tones).
    AgfaCp30,
    /// Standard Kodak Cineon 10-bit Log density curve (`95/1023` black, `685/1023` white).
    CineonLog,
}

/// Parametric sensitometric H&D curve and 3-layer dye cross-matrix parameters for a film stock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilmParams {
    /// 3-layer subtractive dye cross-talk matrix applied in linear exposure space.
    pub dye_matrix: [[f32; 3]; 3],
    /// Per-channel H&D sensitometric gamma (slope in log-exposure space).
    pub gamma: [f32; 3],
    /// Minimum density (`Dmin`) per channel.
    pub dmin: [f32; 3],
    /// Maximum density (`Dmax`) per channel.
    pub dmax: [f32; 3],
    /// Linear-light shadow toe softness.
    pub toe: f32,
    /// Log10-exposure midpoint shift.
    pub exposure_shift: f32,
    /// Normalized black floor at zero exposure (`[R, G, B]`).
    pub black_floor: [f32; 3],
    /// Normalized white ceiling at 1.0 exposure (`[R, G, B]`).
    pub white_ceil: [f32; 3],
    /// Physical backing scatter halation intensity (`0.0..=1.0`).
    pub halation_strength: f32,
    /// Physical silver-halide grain intensity (`0.0..=1.0`).
    pub grain_strength: f32,
}

impl FilmStock {
    /// All 12 built-in physical film stock presets.
    pub const ALL: [Self; 12] = [
        Self::Kodak2383Print,
        Self::Kodak2393Print,
        Self::KodakVision3_5219,
        Self::Portra160,
        Self::Portra400,
        Self::Portra800,
        Self::Ektar100,
        Self::Gold200,
        Self::Fuji3510Print,
        Self::Fuji3513Di,
        Self::AgfaCp30,
        Self::CineonLog,
    ];

    /// Human-readable stock name.
    #[inline]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Kodak2383Print => "Kodak 2383 Print",
            Self::Kodak2393Print => "Kodak 2393 Premier Print",
            Self::KodakVision3_5219 => "Kodak Vision3 500T 5219",
            Self::Portra160 => "Kodak Portra 160",
            Self::Portra400 => "Kodak Portra 400",
            Self::Portra800 => "Kodak Portra 800",
            Self::Ektar100 => "Kodak Ektar 100",
            Self::Gold200 => "Kodak Gold 200",
            Self::Fuji3510Print => "Fujifilm 3510 Print",
            Self::Fuji3513Di => "Fujifilm 3513DI Print",
            Self::AgfaCp30 => "Agfacolor CP30",
            Self::CineonLog => "Kodak Cineon Log",
        }
    }

    /// Returns the sensitometric and dye-matrix parameters for this stock.
    pub const fn params(self) -> FilmParams {
        match self {
            Self::Kodak2383Print => FilmParams {
                dye_matrix: [
                    [1.06, -0.04, -0.02],
                    [-0.03, 1.05, -0.02],
                    [-0.04, 0.02, 1.02],
                ],
                gamma: [1.24, 1.26, 1.22],
                dmin: [0.05, 0.06, 0.08],
                dmax: [2.45, 2.40, 2.35],
                toe: 0.0035,
                exposure_shift: 0.64,
                black_floor: [0.001, 0.003, 0.006],
                white_ceil: [0.985, 0.968, 0.935],
                halation_strength: 0.25,
                grain_strength: 0.18,
            },
            Self::Kodak2393Print => FilmParams {
                dye_matrix: [
                    [1.12, -0.08, -0.04],
                    [-0.05, 1.10, -0.05],
                    [-0.05, -0.02, 1.07],
                ],
                gamma: [1.34, 1.36, 1.32],
                dmin: [0.04, 0.05, 0.07],
                dmax: [2.65, 2.60, 2.55],
                toe: 0.0025,
                exposure_shift: 0.66,
                black_floor: [0.0005, 0.0015, 0.0035],
                white_ceil: [0.980, 0.965, 0.945],
                halation_strength: 0.22,
                grain_strength: 0.16,
            },
            Self::KodakVision3_5219 => FilmParams {
                dye_matrix: [
                    [1.02, 0.01, -0.03],
                    [-0.02, 1.03, -0.01],
                    [0.01, 0.03, 0.96],
                ],
                gamma: [0.94, 0.96, 0.98],
                dmin: [0.07, 0.07, 0.06],
                dmax: [2.05, 2.02, 2.00],
                toe: 0.0080,
                exposure_shift: 0.54,
                black_floor: [0.008, 0.010, 0.014],
                white_ceil: [0.955, 0.962, 0.985],
                halation_strength: 0.52,
                grain_strength: 0.45,
            },
            Self::Portra160 => FilmParams {
                dye_matrix: [
                    [1.03, 0.02, -0.05],
                    [-0.02, 1.02, 0.00],
                    [-0.03, 0.04, 0.99],
                ],
                gamma: [0.92, 0.93, 0.90],
                dmin: [0.06, 0.07, 0.08],
                dmax: [1.95, 1.92, 1.90],
                toe: 0.0070,
                exposure_shift: 0.54,
                black_floor: [0.007, 0.008, 0.007],
                white_ceil: [0.985, 0.968, 0.942],
                halation_strength: 0.20,
                grain_strength: 0.16,
            },
            Self::Portra400 => FilmParams {
                dye_matrix: [
                    [1.05, 0.02, -0.07],
                    [-0.02, 1.03, -0.01],
                    [-0.03, 0.05, 0.98],
                ],
                gamma: [0.96, 0.97, 0.94],
                dmin: [0.06, 0.07, 0.08],
                dmax: [2.05, 2.02, 1.98],
                toe: 0.0065,
                exposure_shift: 0.56,
                black_floor: [0.006, 0.008, 0.006],
                white_ceil: [0.988, 0.970, 0.938],
                halation_strength: 0.30,
                grain_strength: 0.32,
            },
            Self::Portra800 => FilmParams {
                dye_matrix: [
                    [1.08, 0.01, -0.09],
                    [-0.03, 1.04, -0.01],
                    [-0.04, 0.06, 0.98],
                ],
                gamma: [1.04, 1.03, 1.00],
                dmin: [0.06, 0.07, 0.09],
                dmax: [2.18, 2.14, 2.08],
                toe: 0.0060,
                exposure_shift: 0.58,
                black_floor: [0.008, 0.011, 0.009],
                white_ceil: [0.990, 0.968, 0.932],
                halation_strength: 0.48,
                grain_strength: 0.54,
            },
            Self::Ektar100 => FilmParams {
                dye_matrix: [
                    [1.16, -0.10, -0.06],
                    [-0.06, 1.12, -0.06],
                    [-0.04, -0.04, 1.08],
                ],
                gamma: [1.16, 1.16, 1.18],
                dmin: [0.04, 0.05, 0.05],
                dmax: [2.35, 2.35, 2.32],
                toe: 0.0035,
                exposure_shift: 0.62,
                black_floor: [0.002, 0.002, 0.004],
                white_ceil: [0.988, 0.980, 0.975],
                halation_strength: 0.18,
                grain_strength: 0.12,
            },
            Self::Gold200 => FilmParams {
                dye_matrix: [
                    [1.09, 0.03, -0.12],
                    [-0.02, 1.04, -0.02],
                    [-0.05, 0.07, 0.98],
                ],
                gamma: [1.05, 1.04, 0.99],
                dmin: [0.05, 0.07, 0.10],
                dmax: [2.20, 2.15, 2.08],
                toe: 0.0055,
                exposure_shift: 0.58,
                black_floor: [0.009, 0.007, 0.004],
                white_ceil: [0.992, 0.968, 0.905],
                halation_strength: 0.42,
                grain_strength: 0.38,
            },
            Self::Fuji3510Print => FilmParams {
                dye_matrix: [
                    [1.00, -0.03, 0.03],
                    [-0.04, 1.10, -0.06],
                    [-0.01, 0.03, 0.98],
                ],
                gamma: [1.08, 1.18, 1.15],
                dmin: [0.05, 0.05, 0.06],
                dmax: [2.25, 2.32, 2.28],
                toe: 0.0055,
                exposure_shift: 0.56,
                black_floor: [0.002, 0.006, 0.007],
                white_ceil: [0.962, 0.985, 0.972],
                halation_strength: 0.24,
                grain_strength: 0.20,
            },
            Self::Fuji3513Di => FilmParams {
                dye_matrix: [
                    [1.07, -0.05, -0.02],
                    [-0.04, 1.08, -0.04],
                    [-0.02, -0.01, 1.03],
                ],
                gamma: [1.26, 1.28, 1.25],
                dmin: [0.04, 0.04, 0.06],
                dmax: [2.50, 2.48, 2.42],
                toe: 0.0030,
                exposure_shift: 0.63,
                black_floor: [0.001, 0.004, 0.007],
                white_ceil: [0.980, 0.978, 0.955],
                halation_strength: 0.20,
                grain_strength: 0.16,
            },
            Self::AgfaCp30 => FilmParams {
                dye_matrix: [
                    [1.05, -0.01, -0.04],
                    [-0.01, 0.97, 0.04],
                    [-0.04, 0.05, 0.99],
                ],
                gamma: [1.12, 1.10, 1.08],
                dmin: [0.06, 0.07, 0.08],
                dmax: [2.25, 2.22, 2.18],
                toe: 0.0050,
                exposure_shift: 0.59,
                black_floor: [0.006, 0.004, 0.006],
                white_ceil: [0.975, 0.955, 0.935],
                halation_strength: 0.34,
                grain_strength: 0.36,
            },
            Self::CineonLog => FilmParams {
                dye_matrix: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                gamma: [0.60, 0.60, 0.60],
                dmin: [0.0, 0.0, 0.0],
                dmax: [2.0, 2.0, 2.0],
                toe: CINEON_LIN_BLACK,
                exposure_shift: 0.0,
                black_floor: [
                    CINEON_REF_BLACK / CINEON_CODE_RANGE,
                    CINEON_REF_BLACK / CINEON_CODE_RANGE,
                    CINEON_REF_BLACK / CINEON_CODE_RANGE,
                ],
                white_ceil: [
                    CINEON_REF_WHITE / CINEON_CODE_RANGE,
                    CINEON_REF_WHITE / CINEON_CODE_RANGE,
                    CINEON_REF_WHITE / CINEON_CODE_RANGE,
                ],
                halation_strength: 0.0,
                grain_strength: 0.0,
            },
        }
    }

    /// Physical halation intensity scalar (`0.0..=1.0`).
    #[inline]
    pub const fn halation_strength(self) -> f32 {
        self.params().halation_strength
    }

    /// Physical silver-halide grain intensity scalar (`0.0..=1.0`).
    #[inline]
    pub const fn grain_strength(self) -> f32 {
        self.params().grain_strength
    }

    /// Evaluates the physical film stock curve on a linear-light RGB triplet (`[0.0, inf)`).
    pub fn apply_linear(self, rgb: [f32; 3]) -> [f32; 3] {
        let clean = [
            if rgb[0].is_finite() { rgb[0].max(0.0) } else { 0.0 },
            if rgb[1].is_finite() { rgb[1].max(0.0) } else { 0.0 },
            if rgb[2].is_finite() { rgb[2].max(0.0) } else { 0.0 },
        ];
        if matches!(self, Self::CineonLog) {
            return [
                linear_to_cineon_log(clean[0]),
                linear_to_cineon_log(clean[1]),
                linear_to_cineon_log(clean[2]),
            ];
        }

        let p = self.params();
        let dyed = mul_mat3(&p.dye_matrix, clean);
        let mut out = [0.0f32; 3];
        for c in 0..3 {
            let x = dyed[c].max(0.0);
            let t0 = hd_transmittance(0.0, p.gamma[c], p.dmin[c], p.dmax[c], p.toe, p.exposure_shift);
            let t1 = hd_transmittance(1.0, p.gamma[c], p.dmin[c], p.dmax[c], p.toe, p.exposure_shift);
            let tv = hd_transmittance(x, p.gamma[c], p.dmin[c], p.dmax[c], p.toe, p.exposure_shift);
            let norm = (tv - t0) / (t1 - t0).max(1e-6);
            out[c] = (p.black_floor[c] + (p.white_ceil[c] - p.black_floor[c]) * norm).max(0.0);
        }
        out
    }

    /// Evaluates the film stock on sRGB-encoded `[0.0, 1.0]` RGB, returning sRGB-encoded RGB.
    pub fn apply_srgb(self, rgb: [f32; 3]) -> [f32; 3] {
        let lin = [
            srgb_to_linear(rgb[0]).max(0.0),
            srgb_to_linear(rgb[1]).max(0.0),
            srgb_to_linear(rgb[2]).max(0.0),
        ];
        let graded = self.apply_linear(lin);
        if matches!(self, Self::CineonLog) {
            graded
        } else {
            [
                linear_to_srgb(graded[0]).clamp(0.0, 1.0),
                linear_to_srgb(graded[1]).clamp(0.0, 1.0),
                linear_to_srgb(graded[2]).clamp(0.0, 1.0),
            ]
        }
    }

    /// Applies this film stock in place across every pixel in `frame` (preserving alpha).
    pub fn apply_frame(self, frame: &mut LinearFrame) {
        for px in &mut frame.pixels {
            let rgb = self.apply_linear(px.rgb());
            *px = RgbaF32::from_rgb(rgb, px.0[3]);
        }
    }

    /// Bakes this film stock into a `size x size x size` 3D LUT over linear `[0.0, 1.0]^3`.
    pub fn bake_lut3d(self, size: usize) -> Lut3D {
        let sz = size.clamp(2, 128);
        let denom = (sz - 1) as f32;
        let mut table = Vec::with_capacity(sz * sz * sz);
        for b in 0..sz {
            let bf = b as f32 / denom;
            for g in 0..sz {
                let gf = g as f32 / denom;
                for r in 0..sz {
                    let rf = r as f32 / denom;
                    let mapped = self.apply_linear([rf, gf, bf]);
                    table.push([
                        mapped[0].clamp(0.0, 1.0),
                        mapped[1].clamp(0.0, 1.0),
                        mapped[2].clamp(0.0, 1.0),
                    ]);
                }
            }
        }
        Lut3D {
            title: Some(self.name().to_string()),
            size: sz,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        }
    }

    /// Bakes this film stock into a `size x size x size` 3D LUT for sRGB-encoded `[0.0, 1.0]^3`.
    pub fn bake_srgb_lut3d(self, size: usize) -> Lut3D {
        let sz = size.clamp(2, 128);
        let denom = (sz - 1) as f32;
        let mut table = Vec::with_capacity(sz * sz * sz);
        for b in 0..sz {
            let bf = b as f32 / denom;
            for g in 0..sz {
                let gf = g as f32 / denom;
                for r in 0..sz {
                    let rf = r as f32 / denom;
                    let mapped = self.apply_srgb([rf, gf, bf]);
                    table.push([
                        mapped[0].clamp(0.0, 1.0),
                        mapped[1].clamp(0.0, 1.0),
                        mapped[2].clamp(0.0, 1.0),
                    ]);
                }
            }
        }
        Lut3D {
            title: Some(self.name().to_string()),
            size: sz,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        }
    }
}

#[inline]
fn hd_transmittance(
    x: f32,
    gamma: f32,
    dmin: f32,
    dmax: f32,
    toe: f32,
    exposure_shift: f32,
) -> f32 {
    let le = log10f(x.max(0.0) + toe) + exposure_shift;
    let density = dmin + (dmax - dmin) / (1.0 + expf(gamma * LN_10 * le));
    powf(10.0, -density)
}

/// Encodes linear light (`[0.0, inf)`, `1.0` = 90% ref white) into normalized Kodak Cineon 10-bit Log `[0.0, 1.0]`.
#[inline]
pub fn linear_to_cineon_log(v: f32) -> f32 {
    if !v.is_finite() || v <= 0.0 {
        return CINEON_REF_BLACK / CINEON_CODE_RANGE;
    }
    let scaled = v * (1.0 - CINEON_LIN_BLACK) + CINEON_LIN_BLACK;
    ((CINEON_REF_WHITE + CINEON_STEP * log10f(scaled)) / CINEON_CODE_RANGE).clamp(0.0, 1.0)
}

/// Decodes normalized Kodak Cineon 10-bit Log `[0.0, 1.0]` back to linear light.
#[inline]
pub fn cineon_log_to_linear(cv: f32) -> f32 {
    if !cv.is_finite() {
        return 0.0;
    }
    let exp_val = (cv * CINEON_CODE_RANGE - CINEON_REF_WHITE) / CINEON_STEP;
    ((powf(10.0, exp_val) - CINEON_LIN_BLACK) / (1.0 - CINEON_LIN_BLACK)).max(0.0)
}
