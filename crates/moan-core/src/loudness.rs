// SPDX-License-Identifier: GPL-3.0-only
//! ITU-R BS.1770-4 / EBU R128 loudness measurement, 4x oversampled True-Peak detection,
//! and delivery target normalization with `-1.0 dBTP` brickwall limiting.
//! Pure `#![no_std]` + `alloc`, zero `unsafe`.

use crate::{
    dsp_math::{abs_f32, round_f32},
    Frame, RATE,
};
use alloc::{vec, vec::Vec};

/// Digital silence floor for LUFS / dBTP reporting.
pub const SILENCE_LUFS: f32 = -120.0;
const ABS_GATE_LUFS: f64 = -70.0;
const REL_GATE_OFFSET_LU: f64 = 10.0;
const K_OFFSET: f64 = -0.691;

const PI_F64: f64 = 3.14159265358979323846;
const TAU_F64: f64 = 6.28318530717958647692;
const FRAC_PI_2_F64: f64 = 1.57079632679489661923;
const LN_2_F64: f64 = 0.69314718055994530942;
const LOG2_E_F64: f64 = 1.44269504088896340736;
const LOG10_2_F64: f64 = 0.30102999566398119521;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoudnessError {
    InvalidSampleRate,
    InvalidTarget,
    Memory,
}

/// Broadcast & streaming loudness delivery targets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeliveryTarget {
    /// Spotify / YouTube / Apple Music streaming (`-14.0 LUFS`, `-1.0 dBTP` ceiling).
    StreamingMinus14,
    /// Apple Podcasts / Spotify Spoken Word (`-16.0 LUFS`, `-1.0 dBTP` ceiling).
    PodcastMinus16,
    /// EBU R128 European Broadcast (`-23.0 LUFS`, `-1.0 dBTP` ceiling).
    BroadcastMinus23,
    /// Film / ATSC A/85 theatrical & drama (`-24.0 LUFS`, `-1.0 dBTP` ceiling).
    FilmMinus24,
    /// Custom loudness target and True-Peak brickwall ceiling.
    Custom {
        target_lufs: f32,
        max_true_peak_dbtp: f32,
    },
}

impl DeliveryTarget {
    pub fn target_lufs(self) -> f32 {
        match self {
            Self::StreamingMinus14 => -14.0,
            Self::PodcastMinus16 => -16.0,
            Self::BroadcastMinus23 => -23.0,
            Self::FilmMinus24 => -24.0,
            Self::Custom { target_lufs, .. } => target_lufs,
        }
    }

    pub fn max_true_peak_dbtp(self) -> f32 {
        match self {
            Self::StreamingMinus14
            | Self::PodcastMinus16
            | Self::BroadcastMinus23
            | Self::FilmMinus24 => -1.0,
            Self::Custom {
                max_true_peak_dbtp, ..
            } => max_true_peak_dbtp,
        }
    }
}

/// Complete ITU-R BS.1770-4 / EBU R128 loudness snapshot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessMeasurement {
    /// Gated integrated loudness (`-70 LKFS` abs + `-10 dB` relative gate) in LUFS.
    pub integrated_lufs: f32,
    /// Latest 400 ms momentary loudness in LUFS.
    pub momentary_lufs: f32,
    /// Maximum 400 ms momentary loudness observed across the stream in LUFS.
    pub momentary_max_lufs: f32,
    /// Latest 3 s short-term loudness in LUFS.
    pub short_term_lufs: f32,
    /// Maximum 3 s short-term loudness observed across the stream in LUFS.
    pub short_term_max_lufs: f32,
    /// EBU Tech 3342 Loudness Range (LRA) in LU.
    pub loudness_range_lu: f32,
    /// 4x oversampled True-Peak amplitude in linear scale.
    pub true_peak_linear: f32,
    /// 4x oversampled True-Peak level in dBTP.
    pub true_peak_dbtp: f32,
}

/// Report returned by [`normalize_to_target`] and [`normalize_f32_to_target`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizationReport {
    pub before: LoudnessMeasurement,
    pub after: LoudnessMeasurement,
    pub gain_applied_db: f32,
    pub limiter_engaged: bool,
}

// --- High-precision no_std f64 math helpers ---

#[inline]
fn sin_f64(mut x: f64) -> f64 {
    let k = (x / TAU_F64 + if x >= 0.0 { 0.5 } else { -0.5 }) as i64 as f64;
    x -= k * TAU_F64;
    if x > FRAC_PI_2_F64 {
        x = PI_F64 - x;
    } else if x < -FRAC_PI_2_F64 {
        x = -PI_F64 - x;
    }
    let z = x * x;
    x * (1.0
        + z * (-0.16666666666666666
            + z * (0.008333333333333333
                + z * (-0.0001984126984126984
                    + z * (0.0000027557319223985893 - z * 0.00000002505210838544172)))))
}

#[inline]
fn cos_f64(x: f64) -> f64 {
    sin_f64(x + FRAC_PI_2_F64)
}

#[inline]
fn tan_f64(x: f64) -> f64 {
    let c = cos_f64(x);
    if c.abs() < 1e-15 {
        if sin_f64(x) >= 0.0 {
            1e15
        } else {
            -1e15
        }
    } else {
        sin_f64(x) / c
    }
}

#[inline]
fn sqrt_f64(x: f64) -> f64 {
    if x <= 0.0 || !x.is_finite() {
        return 0.0;
    }
    let mut y = crate::dsp_math::sqrt_f32(x as f32) as f64;
    if y <= 0.0 {
        y = 1.0;
    }
    for _ in 0..4 {
        y = 0.5 * (y + x / y);
    }
    y
}

#[inline]
fn log2_f64(x: f64) -> f64 {
    if !x.is_finite() || x <= 0.0 {
        return f64::NEG_INFINITY;
    }
    let bits = x.to_bits();
    let exp = ((bits >> 52) as i64) - 1023;
    let mant = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | 0x3FF0_0000_0000_0000);
    let u = (mant - 1.0) / (mant + 1.0);
    let u2 = u * u;
    let ln_mant = 2.0
        * u
        * (1.0
            + u2 * (1.0 / 3.0
                + u2 * (1.0 / 5.0
                    + u2 * (1.0 / 7.0 + u2 * (1.0 / 9.0 + u2 * (1.0 / 11.0 + u2 / 13.0))))));
    (exp as f64) + ln_mant * LOG2_E_F64
}

#[inline]
fn log10_f64(x: f64) -> f64 {
    log2_f64(x) * LOG10_2_F64
}

#[inline]
fn exp2_f64(x: f64) -> f64 {
    if x <= -1020.0 {
        return 0.0;
    }
    if x >= 1020.0 {
        return f64::INFINITY;
    }
    let fl = if x < 0.0 {
        let i = x as i64;
        if (i as f64) > x {
            (i - 1) as f64
        } else {
            i as f64
        }
    } else {
        (x as i64) as f64
    };
    let i = fl as i64;
    let f = x - fl;
    let z = f * LN_2_F64;
    let p = 1.0
        + z * (1.0
            + z * (0.5
                + z * (1.0 / 6.0
                    + z * (1.0 / 24.0
                        + z * (1.0 / 120.0
                            + z * (1.0 / 720.0 + z * (1.0 / 5040.0 + z / 40320.0)))))));
    let scale = f64::from_bits(((i + 1023) as u64) << 52);
    p * scale
}

#[inline]
fn pow10_f64(x: f64) -> f64 {
    exp2_f64(x / LOG10_2_F64)
}

#[inline]
fn bessel_i0(x: f64) -> f64 {
    let half = 0.5 * x;
    let mut sum = 1.0;
    let mut term = 1.0;
    for k in 1..=16 {
        let kf = k as f64;
        term *= (half / kf) * (half / kf);
        sum += term;
    }
    sum
}

#[inline]
fn mean_sq_to_lufs(mean_sq: f64) -> f32 {
    if !mean_sq.is_finite() || mean_sq <= 1e-12 {
        return SILENCE_LUFS;
    }
    let lufs = (K_OFFSET + 10.0 * log10_f64(mean_sq)) as f32;
    if lufs < SILENCE_LUFS {
        SILENCE_LUFS
    } else {
        lufs
    }
}

#[inline]
fn linear_to_dbtp(lin: f32) -> f32 {
    let a = abs_f32(lin);
    if !a.is_finite() || a <= 1e-6 {
        return SILENCE_LUFS;
    }
    (20.0 * log10_f64(a as f64)) as f32
}

#[derive(Clone, Debug)]
struct KWeightBiquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; 2],
    z2: [f64; 2],
}

impl KWeightBiquad {
    /// ITU-R BS.1770-4 Stage 1 high-shelf pre-filter (+4 dB above ~1.68 kHz).
    fn stage1(sample_rate: u32) -> Self {
        if sample_rate == 48_000 {
            return Self {
                b0: 1.53512485958697,
                b1: -2.69169618940638,
                b2: 1.19839281085285,
                a1: -1.69065929318241,
                a2: 0.73248077421585,
                z1: [0.0; 2],
                z2: [0.0; 2],
            };
        }
        let fs = (sample_rate as f64).max(8000.0);
        let f0 = 1681.974450955533;
        let g_db = 3.999843853973347;
        let q = 0.7071752369554196;
        let k = tan_f64(PI_F64 * f0 / fs);
        let vh = pow10_f64(g_db / 20.0);
        let vb = exp2_f64(0.4996667741545416 * log2_f64(vh));
        let a0 = 1.0 + k / q + k * k;
        Self {
            b0: (vh + vb * k / q + k * k) / a0,
            b1: 2.0 * (k * k - vh) / a0,
            b2: (vh - vb * k / q + k * k) / a0,
            a1: 2.0 * (k * k - 1.0) / a0,
            a2: (1.0 - k / q + k * k) / a0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    /// ITU-R BS.1770-4 Stage 2 RLB high-pass filter (~38.1 Hz).
    fn stage2(sample_rate: u32) -> Self {
        if sample_rate == 48_000 {
            return Self {
                b0: 1.0,
                b1: -2.0,
                b2: 1.0,
                a1: -1.99004745483398,
                a2: 0.99007225036621,
                z1: [0.0; 2],
                z2: [0.0; 2],
            };
        }
        let fs = (sample_rate as f64).max(8000.0);
        let f0 = 38.13547087602444;
        let q = 0.5003270373238773;
        let k = tan_f64(PI_F64 * f0 / fs);
        let a0 = 1.0 + k / q + k * k;
        Self {
            b0: 1.0,
            b1: -2.0,
            b2: 1.0,
            a1: 2.0 * (k * k - 1.0) / a0,
            a2: (1.0 - k / q + k * k) / a0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    #[inline]
    fn process(&mut self, x: [f64; 2]) -> [f64; 2] {
        let mut out = [0.0; 2];
        for c in 0..2 {
            let v = if x[c].is_finite() { x[c] } else { 0.0 };
            let y = self.b0 * v + self.z1[c];
            self.z1[c] = self.b1 * v - self.a1 * y + self.z2[c];
            self.z2[c] = self.b2 * v - self.a2 * y;
            out[c] = y;
        }
        out
    }

    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
}

/// 4x oversampled True-Peak detector using a 4-phase, 12-tap Kaiser-windowed sinc polyphase FIR
/// filter per ITU-R BS.1770-4 Annex 2.
#[derive(Clone, Debug)]
pub struct TruePeakDetector {
    phases: [[f32; 12]; 3],
    history: [[f32; 12]; 2],
    max_peak: f32,
}

impl TruePeakDetector {
    pub fn new() -> Self {
        let beta = 5.2;
        let i0_beta = bessel_i0(beta);
        let mut phases = [[0.0f32; 12]; 3];

        for (p_idx, phase) in phases.iter_mut().enumerate() {
            let frac = (p_idx + 1) as f64 * 0.25;
            let mut sum = 0.0f64;
            let mut tmp = [0.0f64; 12];
            for (i, coeff) in tmp.iter_mut().enumerate() {
                let m = (i as f64 - 5.0) - frac;
                let r = m / 6.0;
                let w = if r.abs() < 1.0 {
                    bessel_i0(beta * sqrt_f64(1.0 - r * r)) / i0_beta
                } else {
                    0.0
                };
                let sinc = if m.abs() < 1e-12 {
                    1.0
                } else {
                    sin_f64(PI_F64 * m) / (PI_F64 * m)
                };
                *coeff = w * sinc;
                sum += *coeff;
            }
            for i in 0..12 {
                phase[i] = (tmp[i] / sum) as f32;
            }
        }

        Self {
            phases,
            history: [[0.0; 12]; 2],
            max_peak: 0.0,
        }
    }

    pub fn reset(&mut self) {
        self.history = [[0.0; 12]; 2];
        self.max_peak = 0.0;
    }

    /// Feeds one stereo `f32` frame and returns the 4x oversampled true-peak amplitude for this step.
    #[inline]
    pub fn process_frame(&mut self, frame: [f32; 2]) -> f32 {
        let mut step_peak = 0.0f32;
        for c in 0..2 {
            let s = if frame[c].is_finite() { frame[c] } else { 0.0 };
            for i in 0..11 {
                self.history[c][i] = self.history[c][i + 1];
            }
            self.history[c][11] = s;

            // Phase 0 (exact center sample at index 5)
            let p0 = abs_f32(self.history[c][5]);
            if p0 > step_peak {
                step_peak = p0;
            }

            // Phases 1..3 (+0.25, +0.50, +0.75)
            for phase in &self.phases {
                let mut acc = 0.0f32;
                for k in 0..12 {
                    acc += self.history[c][k] * phase[k];
                }
                let pa = abs_f32(acc);
                if pa > step_peak {
                    step_peak = pa;
                }
            }
        }
        if step_peak > self.max_peak {
            self.max_peak = step_peak;
        }
        step_peak
    }

    /// Flushes the 6-sample FIR tail with silence so peaks near buffer end are evaluated across all phases.
    pub fn flush(&mut self) -> f32 {
        for _ in 0..6 {
            self.process_frame([0.0, 0.0]);
        }
        self.max_peak
    }

    pub fn max_true_peak_linear(&self) -> f32 {
        self.max_peak
    }

    pub fn max_true_peak_dbtp(&self) -> f32 {
        linear_to_dbtp(self.max_peak)
    }

    /// Measures 4x oversampled True-Peak across a stereo `f32` slice.
    pub fn measure_slice(frames: &[[f32; 2]]) -> f32 {
        let mut det = Self::new();
        for &f in frames {
            det.process_frame(f);
        }
        det.flush()
    }
}

impl Default for TruePeakDetector {
    fn default() -> Self {
        Self::new()
    }
}

/// Stateful ITU-R BS.1770-4 / EBU R128 loudness meter supporting Momentary (400 ms),
/// Short-Term (3 s), Gated Integrated LUFS (`-70 LKFS` / `-10 dB`), LRA, and True-Peak dBTP.
#[derive(Clone, Debug)]
pub struct LoudnessMeter {
    sample_rate: u32,
    sub_block_frames: usize,
    stage1: KWeightBiquad,
    stage2: KWeightBiquad,
    true_peak: TruePeakDetector,
    // Current 100 ms sub-block accumulator
    cur_sum_sq: f64,
    cur_frames: usize,
    // Completed 100 ms sub-block mean-square values (L+R sum per frame)
    sub_blocks: Vec<f64>,
    // 400 ms momentary and 3 s short-term tracking
    latest_momentary_lufs: f32,
    max_momentary_lufs: f32,
    latest_short_term_lufs: f32,
    max_short_term_lufs: f32,
    // Fallback accumulator if total input < 400 ms
    total_sum_sq: f64,
    total_frames: u64,
}

impl LoudnessMeter {
    /// Creates a BS.1770-4 / EBU R128 meter at 48,000 Hz.
    pub fn new() -> Self {
        Self::with_sample_rate(RATE).expect("48 kHz is always valid")
    }

    pub fn with_sample_rate(sample_rate: u32) -> Result<Self, LoudnessError> {
        if !(8000..=192_000).contains(&sample_rate) {
            return Err(LoudnessError::InvalidSampleRate);
        }
        let sub_block_frames = (sample_rate as usize / 10).max(1); // 100 ms hop
        Ok(Self {
            sample_rate,
            sub_block_frames,
            stage1: KWeightBiquad::stage1(sample_rate),
            stage2: KWeightBiquad::stage2(sample_rate),
            true_peak: TruePeakDetector::new(),
            cur_sum_sq: 0.0,
            cur_frames: 0,
            sub_blocks: Vec::new(),
            latest_momentary_lufs: SILENCE_LUFS,
            max_momentary_lufs: SILENCE_LUFS,
            latest_short_term_lufs: SILENCE_LUFS,
            max_short_term_lufs: SILENCE_LUFS,
            total_sum_sq: 0.0,
            total_frames: 0,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn reset(&mut self) {
        self.stage1.reset();
        self.stage2.reset();
        self.true_peak.reset();
        self.cur_sum_sq = 0.0;
        self.cur_frames = 0;
        self.sub_blocks.clear();
        self.latest_momentary_lufs = SILENCE_LUFS;
        self.max_momentary_lufs = SILENCE_LUFS;
        self.latest_short_term_lufs = SILENCE_LUFS;
        self.max_short_term_lufs = SILENCE_LUFS;
        self.total_sum_sq = 0.0;
        self.total_frames = 0;
    }

    #[inline]
    pub fn process_frame_f32(&mut self, frame: [f32; 2]) {
        self.true_peak.process_frame(frame);
        let x = [
            if frame[0].is_finite() { frame[0] as f64 } else { 0.0 },
            if frame[1].is_finite() { frame[1] as f64 } else { 0.0 },
        ];
        let s1 = self.stage1.process(x);
        let s2 = self.stage2.process(s1);
        let sq = s2[0] * s2[0] + s2[1] * s2[1];

        self.cur_sum_sq += sq;
        self.cur_frames += 1;
        self.total_sum_sq += sq;
        self.total_frames += 1;

        if self.cur_frames == self.sub_block_frames {
            let block_mean = self.cur_sum_sq / (self.sub_block_frames as f64);
            self.sub_blocks.push(block_mean);
            self.cur_sum_sq = 0.0;
            self.cur_frames = 0;

            let n = self.sub_blocks.len();
            if n >= 4 {
                let m_mean = 0.25 * self.sub_blocks[n - 4..n].iter().sum::<f64>();
                let m_lufs = mean_sq_to_lufs(m_mean);
                self.latest_momentary_lufs = m_lufs;
                if m_lufs > self.max_momentary_lufs {
                    self.max_momentary_lufs = m_lufs;
                }
            }
            if n >= 30 {
                let s_mean = self.sub_blocks[n - 30..n].iter().sum::<f64>() / 30.0;
                let s_lufs = mean_sq_to_lufs(s_mean);
                self.latest_short_term_lufs = s_lufs;
                if s_lufs > self.max_short_term_lufs {
                    self.max_short_term_lufs = s_lufs;
                }
            } else {
                let s_mean = self.sub_blocks.iter().sum::<f64>() / (n as f64);
                let s_lufs = mean_sq_to_lufs(s_mean);
                self.latest_short_term_lufs = s_lufs;
                if s_lufs > self.max_short_term_lufs {
                    self.max_short_term_lufs = s_lufs;
                }
            }
        }
    }

    pub fn process_f32(&mut self, frames: &[[f32; 2]]) {
        for &f in frames {
            self.process_frame_f32(f);
        }
    }

    pub fn process_frames(&mut self, frames: &[Frame]) {
        for &f in frames {
            self.process_frame_f32([f[0] as f32 / 32768.0, f[1] as f32 / 32768.0]);
        }
    }

    pub fn momentary_lufs(&self) -> f32 {
        if self.sub_blocks.len() >= 4 {
            self.latest_momentary_lufs
        } else if self.total_frames > 0 {
            mean_sq_to_lufs(self.total_sum_sq / (self.total_frames as f64))
        } else {
            SILENCE_LUFS
        }
    }

    pub fn short_term_lufs(&self) -> f32 {
        if !self.sub_blocks.is_empty() {
            self.latest_short_term_lufs
        } else if self.total_frames > 0 {
            mean_sq_to_lufs(self.total_sum_sq / (self.total_frames as f64))
        } else {
            SILENCE_LUFS
        }
    }

    /// Computes ITU-R BS.1770-4 two-stage gated integrated loudness (`-70 LKFS` / `-10 dB` relative).
    /// Falls back to ungated K-weighted loudness if the stream is shorter than 400 ms.
    pub fn integrated_lufs(&self) -> f32 {
        if self.total_frames == 0 {
            return SILENCE_LUFS;
        }
        if self.sub_blocks.len() < 4 {
            let lufs = mean_sq_to_lufs(self.total_sum_sq / (self.total_frames as f64));
            return if lufs <= ABS_GATE_LUFS as f32 {
                SILENCE_LUFS
            } else {
                lufs
            };
        }

        // Build all overlapping 400 ms gating blocks (4 consecutive 100 ms sub-blocks).
        let abs_thresh = pow10_f64((ABS_GATE_LUFS - K_OFFSET) / 10.0);
        let mut abs_sum = 0.0f64;
        let mut abs_count = 0usize;

        for w in self.sub_blocks.windows(4) {
            let z = 0.25 * (w[0] + w[1] + w[2] + w[3]);
            if z > abs_thresh {
                abs_sum += z;
                abs_count += 1;
            }
        }
        if abs_count == 0 {
            return SILENCE_LUFS;
        }

        let gamma_r_lufs = K_OFFSET + 10.0 * log10_f64(abs_sum / (abs_count as f64)) - REL_GATE_OFFSET_LU;
        let rel_thresh = pow10_f64((gamma_r_lufs - K_OFFSET) / 10.0);

        let mut rel_sum = 0.0f64;
        let mut rel_count = 0usize;
        for w in self.sub_blocks.windows(4) {
            let z = 0.25 * (w[0] + w[1] + w[2] + w[3]);
            if z > abs_thresh && z > rel_thresh {
                rel_sum += z;
                rel_count += 1;
            }
        }
        if rel_count == 0 {
            return SILENCE_LUFS;
        }
        mean_sq_to_lufs(rel_sum / (rel_count as f64))
    }

    /// EBU Tech 3342 Loudness Range (LRA) over 3 s short-term blocks.
    pub fn loudness_range_lu(&self) -> f32 {
        if self.sub_blocks.len() < 30 {
            return 0.0;
        }
        let abs_thresh = pow10_f64((-70.0 - K_OFFSET) / 10.0);
        let mut st_powers: Vec<f64> = Vec::new();
        let mut abs_sum = 0.0f64;
        for w in self.sub_blocks.windows(30) {
            let z = w.iter().sum::<f64>() / 30.0;
            if z > abs_thresh {
                st_powers.push(z);
                abs_sum += z;
            }
        }
        if st_powers.len() < 2 {
            return 0.0;
        }
        let ungated = K_OFFSET + 10.0 * log10_f64(abs_sum / (st_powers.len() as f64));
        let rel_thresh = pow10_f64(((ungated - 20.0) - K_OFFSET) / 10.0);
        let mut gated: Vec<f64> = st_powers
            .into_iter()
            .filter(|&z| z > rel_thresh)
            .map(|z| K_OFFSET + 10.0 * log10_f64(z))
            .collect();
        if gated.len() < 2 {
            return 0.0;
        }
        gated.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let p10 = gated[(gated.len() - 1) * 10 / 100];
        let p95 = gated[(gated.len() - 1) * 95 / 100];
        (p95 - p10).max(0.0) as f32
    }

    pub fn true_peak_linear(&self) -> f32 {
        self.true_peak.max_true_peak_linear()
    }

    pub fn true_peak_dbtp(&self) -> f32 {
        self.true_peak.max_true_peak_dbtp()
    }

    /// Finalizes True-Peak tail evaluation and returns a full [`LoudnessMeasurement`].
    pub fn finish(&mut self) -> LoudnessMeasurement {
        self.true_peak.flush();
        self.summary()
    }

    pub fn summary(&self) -> LoudnessMeasurement {
        let momentary = self.momentary_lufs();
        let short_term = self.short_term_lufs();
        LoudnessMeasurement {
            integrated_lufs: self.integrated_lufs(),
            momentary_lufs: momentary,
            momentary_max_lufs: self.max_momentary_lufs.max(momentary),
            short_term_lufs: short_term,
            short_term_max_lufs: self.max_short_term_lufs.max(short_term),
            loudness_range_lu: self.loudness_range_lu(),
            true_peak_linear: self.true_peak_linear(),
            true_peak_dbtp: self.true_peak_dbtp(),
        }
    }
}

impl Default for LoudnessMeter {
    fn default() -> Self {
        Self::new()
    }
}

/// Measures ITU-R BS.1770-4 / EBU R128 loudness and 4x True-Peak on 48 kHz stereo `Frame`s.
pub fn measure_frames(frames: &[Frame]) -> LoudnessMeasurement {
    let mut meter = LoudnessMeter::new();
    meter.process_frames(frames);
    meter.finish()
}

/// Measures ITU-R BS.1770-4 / EBU R128 loudness and 4x True-Peak on stereo `f32` frames.
pub fn measure_f32(frames: &[[f32; 2]], sample_rate: u32) -> Result<LoudnessMeasurement, LoudnessError> {
    let mut meter = LoudnessMeter::with_sample_rate(sample_rate)?;
    meter.process_f32(frames);
    Ok(meter.finish())
}

/// Applies lookahead 4x True-Peak brickwall limiting to `buf` at `ceiling_lin`. Returns `true` if engaged.
fn apply_true_peak_brickwall(
    buf: &mut [[f32; 2]],
    sample_rate: u32,
    ceiling_lin: f32,
) -> Result<bool, LoudnessError> {
    if buf.is_empty() {
        return Ok(false);
    }

    let n = buf.len();
    let mut det = TruePeakDetector::new();
    let mut req_gain = Vec::new();
    req_gain
        .try_reserve_exact(n)
        .map_err(|_| LoudnessError::Memory)?;

    // Center sample of the 12-tap FIR sits at index 5 (5-sample group delay).
    // Feed 5 leading frames first, then align each frame i's true-peak measurement with i.
    let mut engaged = false;
    let lead = 5.min(n);
    for frame in buf.iter().take(lead) {
        det.process_frame(*frame);
    }
    for i in 0..n {
        let step_tp = if i + 5 < n {
            det.process_frame(buf[i + 5])
        } else {
            det.process_frame([0.0, 0.0])
        };
        let g = if step_tp > ceiling_lin && step_tp > 1e-9 {
            engaged = true;
            ceiling_lin / step_tp
        } else {
            1.0
        };
        req_gain.push(g);
    }

    if !engaged {
        // Double-check exact full-buffer true peak (including tail flush).
        let peak = TruePeakDetector::measure_slice(buf);
        if peak <= ceiling_lin {
            return Ok(false);
        }
        let scale = (ceiling_lin * 0.999) / peak;
        for f in buf.iter_mut() {
            f[0] *= scale;
            f[1] *= scale;
        }
        return Ok(true);
    }

    // Backward lookahead attack ramp (~1.5 ms) so gain drops smoothly BEFORE any transient.
    let attack_frames = ((sample_rate as f32 * 0.0015) as usize).max(16);
    let attack_step = 1.0f32 / (attack_frames as f32);
    for i in (0..n.saturating_sub(1)).rev() {
        let next = req_gain[i + 1] + attack_step;
        if next < req_gain[i] {
            req_gain[i] = next;
        }
    }

    // Forward exponential release (~55 ms) maintaining gain <= req_gain[i] everywhere.
    let rel_coeff = crate::dsp_math::exp_f32(-1.0 / (0.055 * sample_rate as f32));
    let mut gr = 0.0f32; // gain reduction in [0, 1)
    for i in 0..n {
        let needed_gr = (1.0 - req_gain[i]).max(0.0);
        gr = (gr * rel_coeff).max(needed_gr);
        let g = (1.0 - gr).clamp(0.0, 1.0);
        buf[i][0] *= g;
        buf[i][1] *= g;
    }

    // Final safety guarantee against residual FIR edge overshoot.
    let final_tp = TruePeakDetector::measure_slice(buf);
    if final_tp > ceiling_lin && final_tp > 1e-9 {
        let trim = (ceiling_lin * 0.998) / final_tp;
        for f in buf.iter_mut() {
            f[0] *= trim;
            f[1] *= trim;
        }
    }

    Ok(true)
}

/// Normalizes stereo `f32` audio in-place to `target` LUFS with a True-Peak brickwall ceiling.
pub fn normalize_f32_to_target(
    frames: &mut [[f32; 2]],
    sample_rate: u32,
    target: DeliveryTarget,
) -> Result<NormalizationReport, LoudnessError> {
    let target_lufs = target.target_lufs();
    let max_tp_dbtp = target.max_true_peak_dbtp();
    if !target_lufs.is_finite()
        || !max_tp_dbtp.is_finite()
        || !(-60.0..=-1.0).contains(&target_lufs)
        || !(-12.0..=0.0).contains(&max_tp_dbtp)
    {
        return Err(LoudnessError::InvalidTarget);
    }

    let before = measure_f32(frames, sample_rate)?;
    if frames.is_empty()
        || before.integrated_lufs <= SILENCE_LUFS + 1.0
        || before.true_peak_linear <= 1e-7
    {
        return Ok(NormalizationReport {
            before,
            after: before,
            gain_applied_db: 0.0,
            limiter_engaged: false,
        });
    }

    let gain_db = (target_lufs - before.integrated_lufs).clamp(-48.0, 48.0);
    let gain_lin = pow10_f64(gain_db as f64 / 20.0) as f32;
    let ceiling_lin = pow10_f64(max_tp_dbtp as f64 / 20.0) as f32;

    for f in frames.iter_mut() {
        f[0] = if f[0].is_finite() { f[0] * gain_lin } else { 0.0 };
        f[1] = if f[1].is_finite() { f[1] * gain_lin } else { 0.0 };
    }

    let mut limiter_engaged = apply_true_peak_brickwall(frames, sample_rate, ceiling_lin)?;
    let mut after = measure_f32(frames, sample_rate)?;

    // If brickwall limiting slightly pulled loudness below target, apply bounded makeup trims.
    for _ in 0..3 {
        let residual_db = target_lufs - after.integrated_lufs;
        if !limiter_engaged || residual_db <= 0.2 || after.integrated_lufs <= SILENCE_LUFS + 1.0 {
            break;
        }
        let makeup_db = residual_db.min(6.0);
        let makeup_lin = pow10_f64(makeup_db as f64 / 20.0) as f32;
        for f in frames.iter_mut() {
            f[0] *= makeup_lin;
            f[1] *= makeup_lin;
        }
        limiter_engaged |= apply_true_peak_brickwall(frames, sample_rate, ceiling_lin)?;
        let next = measure_f32(frames, sample_rate)?;
        if next.integrated_lufs <= after.integrated_lufs + 0.05 {
            after = next;
            break;
        }
        after = next;
    }

    Ok(NormalizationReport {
        before,
        after,
        gain_applied_db: gain_db,
        limiter_engaged,
    })
}

/// Normalizes 48 kHz stereo `Frame`s in-place to `target` LUFS with `-1.0 dBTP` brickwall ceiling.
pub fn normalize_to_target(
    frames: &mut [Frame],
    target: DeliveryTarget,
) -> Result<NormalizationReport, LoudnessError> {
    let target_lufs = target.target_lufs();
    let max_tp_dbtp = target.max_true_peak_dbtp();
    if !target_lufs.is_finite()
        || !max_tp_dbtp.is_finite()
        || !(-60.0..=-1.0).contains(&target_lufs)
        || !(-12.0..=0.0).contains(&max_tp_dbtp)
    {
        return Err(LoudnessError::InvalidTarget);
    }

    let before = measure_frames(frames);
    if frames.is_empty()
        || before.integrated_lufs <= SILENCE_LUFS + 1.0
        || before.true_peak_linear <= 1e-7
    {
        return Ok(NormalizationReport {
            before,
            after: before,
            gain_applied_db: 0.0,
            limiter_engaged: false,
        });
    }

    let mut work = Vec::new();
    work.try_reserve_exact(frames.len())
        .map_err(|_| LoudnessError::Memory)?;
    for &f in frames.iter() {
        work.push([f[0] as f32 / 32768.0, f[1] as f32 / 32768.0]);
    }

    let mut report = normalize_f32_to_target(&mut work, RATE, target)?;

    // Account for i16 quantization safety margin so quantized PCM never breaches True-Peak ceiling.
    let ceiling_lin = pow10_f64(max_tp_dbtp as f64 / 20.0) as f32;
    let q_safety = ceiling_lin * (32766.0 / 32768.0);
    if report.after.true_peak_linear > q_safety && report.after.true_peak_linear > 1e-9 {
        let trim = q_safety / report.after.true_peak_linear;
        for f in &mut work {
            f[0] *= trim;
            f[1] *= trim;
        }
    }

    for (dst, src) in frames.iter_mut().zip(work.iter()) {
        *dst = [
            round_f32(src[0] * 32768.0).clamp(-32768.0, 32767.0) as i16,
            round_f32(src[1] * 32768.0).clamp(-32768.0, 32767.0) as i16,
        ];
    }

    // Ensure quantized frames strictly respect ceiling even under rounding.
    let final_check = measure_frames(frames);
    if final_check.true_peak_linear > ceiling_lin && final_check.true_peak_linear > 1e-9 {
        let scale = (ceiling_lin * 0.998) / final_check.true_peak_linear;
        for f in frames.iter_mut() {
            f[0] = round_f32(f[0] as f32 * scale).clamp(-32768.0, 32767.0) as i16;
            f[1] = round_f32(f[1] as f32 * scale).clamp(-32768.0, 32767.0) as i16;
        }
        report.limiter_engaged = true;
        report.after = measure_frames(frames);
    } else {
        report.after = final_check;
    }

    Ok(report)
}

/// Keeps `vec![0.0]` import clean for internal tests if needed.
#[allow(dead_code)]
fn _zero_pad(len: usize) -> Vec<[f32; 2]> {
    vec![[0.0, 0.0]; len]
}
