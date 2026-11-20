//! 4-way broadcast color wheels (Lift / Gamma / Gain / Offset), exposure,
//! temperature/tint, pivot contrast, luma-preserving saturation & vibrance,
//! and Fritsch-Carlson monotonic piecewise cubic splines (`Curve1D`).

use alloc::vec;
use alloc::vec::Vec;
use libm::{copysignf, exp2f, fabsf, floorf, powf, sqrtf};

use crate::frame::{LinearFrame, RgbaF32};
use crate::lut::Lut3D;
use crate::space::ColorPrimaries;

/// Piecewise monotonic cubic Hermite spline (Fritsch-Carlson PCHIP).
///
/// Guarantees `C1` continuity and zero overshoot on monotonic control points.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve1D {
    points: Vec<[f32; 2]>,
    tangents: Vec<f32>,
}

impl Default for Curve1D {
    fn default() -> Self {
        Self::identity()
    }
}

impl Curve1D {
    /// Standard `[0.0, 1.0]` identity curve `f(x) = x`.
    pub fn identity() -> Self {
        Self {
            points: vec![[0.0, 0.0], [1.0, 1.0]],
            tangents: vec![1.0, 1.0],
        }
    }

    /// Constant flat curve `f(x) = value` over `[0.0, 1.0]` (used as default for `hue_vs_sat` at `1.0`).
    pub fn flat(value: f32) -> Self {
        let v = if value.is_finite() { value } else { 1.0 };
        Self {
            points: vec![[0.0, v], [1.0, v]],
            tangents: vec![0.0, 0.0],
        }
    }

    /// Constructs a piecewise monotonic spline from `(x, y)` control points.
    ///
    /// Points are sorted by `x` and non-finite entries are ignored.
    pub fn from_points(raw_points: &[[f32; 2]]) -> Self {
        let mut pts: Vec<[f32; 2]> = raw_points
            .iter()
            .copied()
            .filter(|p| p[0].is_finite() && p[1].is_finite())
            .collect();
        if pts.is_empty() {
            return Self::identity();
        }
        pts.sort_by(|a, b| a[0].total_cmp(&b[0]));

        // Deduplicate identical x knots (keep latest y).
        let mut dedup: Vec<[f32; 2]> = Vec::with_capacity(pts.len());
        for p in pts {
            if let Some(last) = dedup.last_mut() {
                if fabsf(p[0] - last[0]) <= 1e-6 {
                    last[1] = p[1];
                    continue;
                }
            }
            dedup.push(p);
        }

        let n = dedup.len();
        if n == 1 {
            return Self {
                points: dedup,
                tangents: vec![0.0],
            };
        }
        if n == 2 {
            let slope = (dedup[1][1] - dedup[0][1]) / (dedup[1][0] - dedup[0][0]);
            return Self {
                points: dedup,
                tangents: vec![slope, slope],
            };
        }

        let mut h = Vec::with_capacity(n - 1);
        let mut d = Vec::with_capacity(n - 1);
        for k in 0..(n - 1) {
            let hk = dedup[k + 1][0] - dedup[k][0];
            let dk = (dedup[k + 1][1] - dedup[k][1]) / hk;
            h.push(hk);
            d.push(dk);
        }

        let mut m = vec![0.0f32; n];
        // Interior tangents via weighted harmonic mean (Fritsch-Butland).
        for k in 1..(n - 1) {
            if d[k - 1] * d[k] > 0.0 {
                let w1 = 2.0 * h[k] + h[k - 1];
                let w2 = h[k] + 2.0 * h[k - 1];
                m[k] = (w1 + w2) / (w1 / d[k - 1] + w2 / d[k]);
            } else {
                m[k] = 0.0;
            }
        }

        // Endpoint tangents with non-overshoot shape clamp.
        m[0] = endpoint_slope(h[0], h[1], d[0], d[1]);
        m[n - 1] = endpoint_slope(h[n - 2], h[n - 3], d[n - 2], d[n - 3]);

        // Fritsch-Carlson monotonicity limiter.
        for k in 0..(n - 1) {
            if fabsf(d[k]) <= 1e-12 {
                m[k] = 0.0;
                m[k + 1] = 0.0;
            } else {
                let alpha = m[k] / d[k];
                let beta = m[k + 1] / d[k];
                let r2 = alpha * alpha + beta * beta;
                if r2 > 9.0 {
                    let tau = 3.0 / sqrtf(r2);
                    m[k] = tau * alpha * d[k];
                    m[k + 1] = tau * beta * d[k];
                }
            }
        }

        Self {
            points: dedup,
            tangents: m,
        }
    }

    /// Returns the sorted control points.
    #[inline]
    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// Returns `true` if this curve is the exact identity `f(x) = x`.
    #[inline]
    pub fn is_identity(&self) -> bool {
        self.points.len() == 2
            && fabsf(self.points[0][0]) < 1e-6
            && fabsf(self.points[0][1]) < 1e-6
            && fabsf(self.points[1][0] - 1.0) < 1e-6
            && fabsf(self.points[1][1] - 1.0) < 1e-6
    }

    /// Returns `true` if this curve is constantly `value`.
    #[inline]
    pub fn is_flat(&self, value: f32) -> bool {
        !self.points.is_empty()
            && self.points.iter().all(|p| fabsf(p[1] - value) < 1e-6)
            && self.tangents.iter().all(|&t| fabsf(t) < 1e-6)
    }

    /// Evaluates the spline at `x`, linearly extending HDR values beyond the upper knot.
    pub fn eval(&self, x: f32) -> f32 {
        if !x.is_finite() {
            return 0.0;
        }
        if self.is_identity() {
            return x;
        }
        let n = self.points.len();
        if n == 0 {
            return x;
        }
        if n == 1 {
            return self.points[0][1];
        }
        if x <= self.points[0][0] {
            return self.points[0][1];
        }
        let last = n - 1;
        if x >= self.points[last][0] {
            let slope = self.tangents[last].max(0.0);
            return self.points[last][1] + slope * (x - self.points[last][0]);
        }

        // Binary search for segment [k, k+1].
        let mut lo = 0usize;
        let mut hi = last;
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if self.points[mid][0] <= x {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        let x0 = self.points[lo][0];
        let x1 = self.points[lo + 1][0];
        let h = x1 - x0;
        let t = (x - x0) / h;
        let t2 = t * t;
        let t3 = t2 * t;
        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        h00 * self.points[lo][1]
            + h10 * h * self.tangents[lo]
            + h01 * self.points[lo + 1][1]
            + h11 * h * self.tangents[lo + 1]
    }

    /// Evaluates the spline periodically over `[0.0, 1.0)` (for `hue_vs_sat`).
    pub fn eval_periodic(&self, x: f32) -> f32 {
        if !x.is_finite() {
            return 1.0;
        }
        let wrapped = x - floorf(x);
        if self.points.is_empty() {
            return 1.0;
        }
        let last = self.points.len() - 1;
        if wrapped >= self.points[last][0] {
            return self.points[last][1];
        }
        self.eval(wrapped)
    }
}

fn endpoint_slope(h0: f32, h1: f32, d0: f32, d1: f32) -> f32 {
    let mut m = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
    if m * d0 <= 0.0 {
        m = 0.0;
    } else if d0 * d1 <= 0.0 && fabsf(m) > 3.0 * fabsf(d0) {
        m = 3.0 * d0;
    } else if fabsf(m) > 3.0 * fabsf(d0) {
        m = 3.0 * d0;
    }
    m
}

/// 4-way broadcast color grading parameters and curves.
///
/// [`ColorGrade::default()`] is an exact identity transform across SDR and HDR ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorGrade {
    /// Shadow lift `[R, G, B]` (default `[0.0; 3]`; anchors white at `1.0`).
    pub lift: [f32; 3],
    /// Midtone gamma `[R, G, B]` (default `[1.0; 3]`; `> 1.0` brightens midtones).
    pub gamma: [f32; 3],
    /// Highlight gain `[R, G, B]` (default `[1.0; 3]`; anchors black at `0.0`).
    pub gain: [f32; 3],
    /// Additive offset `[R, G, B]` (default `[0.0; 3]`).
    pub offset: [f32; 3],
    /// Power-law contrast around [`Self::pivot`] (default `1.0`).
    pub contrast: f32,
    /// Linear-light contrast pivot (default `0.18`).
    pub pivot: f32,
    /// Luma-preserving saturation multiplier (default `1.0`, `0.0` = grayscale).
    pub saturation: f32,
    /// Luma-preserving selective vibrance (default `0.0`; boosts muted colors while protecting vivid ones).
    pub vibrance: f32,
    /// White-balance warmth/coolness (`0.0` = neutral, positive = warmer, negative = cooler).
    pub temperature: f32,
    /// White-balance green/magenta tint (`0.0` = neutral, positive = magenta, negative = green).
    pub tint: f32,
    /// Exposure adjustment in photographic stops EV (`0.0` = `1.0x`, `+1.0` = `2.0x`).
    pub exposure_ev: f32,
    /// Master tonal spline applied equally to R, G, and B.
    pub master: Curve1D,
    /// Red channel spline.
    pub r: Curve1D,
    /// Green channel spline.
    pub g: Curve1D,
    /// Blue channel spline.
    pub b: Curve1D,
    /// Hue-vs-saturation multiplier spline (`x in [0, 1)` hue -> saturation multiplier, default flat `1.0`).
    pub hue_vs_sat: Curve1D,
}

impl Default for ColorGrade {
    fn default() -> Self {
        Self {
            lift: [0.0, 0.0, 0.0],
            gamma: [1.0, 1.0, 1.0],
            gain: [1.0, 1.0, 1.0],
            offset: [0.0, 0.0, 0.0],
            contrast: 1.0,
            pivot: 0.18,
            saturation: 1.0,
            vibrance: 0.0,
            temperature: 0.0,
            tint: 0.0,
            exposure_ev: 0.0,
            master: Curve1D::identity(),
            r: Curve1D::identity(),
            g: Curve1D::identity(),
            b: Curve1D::identity(),
            hue_vs_sat: Curve1D::flat(1.0),
        }
    }
}

impl ColorGrade {
    /// Creates a neutral identity grade.
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates the color grade on a linear RGB triplet using the specified primaries for luma.
    pub fn eval_rgb(&self, rgb: [f32; 3], primaries: ColorPrimaries) -> [f32; 3] {
        let exp_scale = if fabsf(self.exposure_ev) > 1e-6 && self.exposure_ev.is_finite() {
            exp2f(self.exposure_ev)
        } else {
            1.0
        };
        let wb = wb_gains(self.temperature, self.tint, primaries);
        let pivot = if self.pivot.is_finite() && self.pivot > 1e-4 {
            self.pivot
        } else {
            0.18
        };
        let contrast = if self.contrast.is_finite() {
            self.contrast.max(0.0)
        } else {
            1.0
        };
        let do_contrast = fabsf(contrast - 1.0) > 1e-6;

        let mut out = [0.0f32; 3];
        for c in 0..3 {
            let mut v = if rgb[c].is_finite() { rgb[c] } else { 0.0 };
            v = v * exp_scale * wb[c];

            let lift = if self.lift[c].is_finite() { self.lift[c] } else { 0.0 };
            let gain = if self.gain[c].is_finite() { self.gain[c] } else { 1.0 };
            let offset = if self.offset[c].is_finite() { self.offset[c] } else { 0.0 };
            if fabsf(lift) > 1e-6 || fabsf(gain - 1.0) > 1e-6 || fabsf(offset) > 1e-6 {
                v = gain * (v + lift * (1.0 - v)) + offset;
            }

            let gamma = if self.gamma[c].is_finite() {
                self.gamma[c].max(0.01)
            } else {
                1.0
            };
            if fabsf(gamma - 1.0) > 1e-6 {
                v = copysignf(powf(fabsf(v), 1.0 / gamma), v);
            }

            if do_contrast && v > 0.0 {
                v = pivot * powf(v / pivot, contrast);
            }
            out[c] = v;
        }

        let [kr, kg, kb] = primaries.luma_weights();
        let luma = kr * out[0] + kg * out[1] + kb * out[2];

        // Luma-preserving saturation.
        let sat_setting = if self.saturation.is_finite() {
            self.saturation.max(0.0)
        } else {
            1.0
        };
        if fabsf(sat_setting - 1.0) > 1e-6 {
            out[0] = luma + (out[0] - luma) * sat_setting;
            out[1] = luma + (out[1] - luma) * sat_setting;
            out[2] = luma + (out[2] - luma) * sat_setting;
        }

        // Luma-preserving vibrance.
        if fabsf(self.vibrance) > 1e-6 && self.vibrance.is_finite() {
            let max_c = out[0].max(out[1]).max(out[2]);
            let min_c = out[0].min(out[1]).min(out[2]);
            let chroma_sat = if max_c > 1e-5 {
                ((max_c - min_c) / max_c).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let vib_mult = (1.0 + self.vibrance * (1.0 - chroma_sat)).max(0.0);
            out[0] = luma + (out[0] - luma) * vib_mult;
            out[1] = luma + (out[1] - luma) * vib_mult;
            out[2] = luma + (out[2] - luma) * vib_mult;
        }

        // Master and per-channel splines.
        if !self.master.is_identity() {
            out[0] = self.master.eval(out[0]);
            out[1] = self.master.eval(out[1]);
            out[2] = self.master.eval(out[2]);
        }
        if !self.r.is_identity() {
            out[0] = self.r.eval(out[0]);
        }
        if !self.g.is_identity() {
            out[1] = self.g.eval(out[1]);
        }
        if !self.b.is_identity() {
            out[2] = self.b.eval(out[2]);
        }

        // Hue vs saturation spline (luma-preserving).
        if !self.hue_vs_sat.is_flat(1.0) {
            if let Some(hue) = rgb_hue(out) {
                let mult = self.hue_vs_sat.eval_periodic(hue).max(0.0);
                let l2 = kr * out[0] + kg * out[1] + kb * out[2];
                out[0] = l2 + (out[0] - l2) * mult;
                out[1] = l2 + (out[1] - l2) * mult;
                out[2] = l2 + (out[2] - l2) * mult;
            }
        }

        out
    }

    /// Evaluates the color grade using [`ColorPrimaries::Rec709`] luma weights.
    #[inline]
    pub fn eval(&self, rgb: [f32; 3]) -> [f32; 3] {
        self.eval_rgb(rgb, ColorPrimaries::Rec709)
    }

    /// Applies this color grade in place to every pixel in `frame` (preserving alpha).
    pub fn apply_frame(&self, frame: &mut LinearFrame) {
        let primaries = frame.primaries;
        for px in &mut frame.pixels {
            let graded = self.eval_rgb(px.rgb(), primaries);
            *px = RgbaF32::from_rgb(graded, px.0[3]);
        }
    }

    /// Bakes this color grade into a `size x size x size` 3D LUT over linear `[0.0, 1.0]^3`.
    pub fn bake_lut3d(&self, size: usize, primaries: ColorPrimaries) -> Lut3D {
        let sz = size.clamp(2, 128);
        let denom = (sz - 1) as f32;
        let mut table = Vec::with_capacity(sz * sz * sz);
        for b in 0..sz {
            let bf = b as f32 / denom;
            for g in 0..sz {
                let gf = g as f32 / denom;
                for r in 0..sz {
                    let rf = r as f32 / denom;
                    let mapped = self.eval_rgb([rf, gf, bf], primaries);
                    table.push([
                        mapped[0].clamp(0.0, 1.0),
                        mapped[1].clamp(0.0, 1.0),
                        mapped[2].clamp(0.0, 1.0),
                    ]);
                }
            }
        }
        Lut3D {
            title: None,
            size: sz,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            table,
        }
    }
}

fn wb_gains(temperature: f32, tint: f32, primaries: ColorPrimaries) -> [f32; 3] {
    let temp = if temperature.is_finite() { temperature } else { 0.0 };
    let tnt = if tint.is_finite() { tint } else { 0.0 };
    if fabsf(temp) <= 1e-6 && fabsf(tnt) <= 1e-6 {
        return [1.0, 1.0, 1.0];
    }
    let r = exp2f(0.45 * temp + 0.22 * tnt);
    let g = exp2f(-0.35 * tnt);
    let b = exp2f(-0.45 * temp + 0.22 * tnt);
    let [kr, kg, kb] = primaries.luma_weights();
    let luma = (kr * r + kg * g + kb * b).max(1e-6);
    [r / luma, g / luma, b / luma]
}

fn rgb_hue(rgb: [f32; 3]) -> Option<f32> {
    let r = rgb[0];
    let g = rgb[1];
    let b = rgb[2];
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let delta = max_c - min_c;
    if delta <= 1e-6 {
        return None;
    }
    let mut h = if (max_c - r).abs() <= 1e-9 {
        (g - b) / delta
    } else if (max_c - g).abs() <= 1e-9 {
        2.0 + (b - r) / delta
    } else {
        4.0 + (r - g) / delta
    };
    h /= 6.0;
    if h < 0.0 {
        h += 1.0;
    }
    Some(h)
}
