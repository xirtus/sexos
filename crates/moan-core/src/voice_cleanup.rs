// SPDX-License-Identifier: GPL-3.0-only
//! OmaAudio Quick Voice Fixes DSP suite for 48 kHz stereo `Frame` (`[i16; 2]`) and `f32` buffers.
//! Pure `#![no_std]` + `alloc`, zero `unsafe`, deterministic finite-arithmetic guarantees.
//!
//! Includes:
//! - [`HumRemover`]: 50 Hz / 60 Hz fundamental + up to 6th harmonic high-Q notch cascade
//! - [`VoiceDenoise`]: 8-band Linkwitz-Riley spectral-envelope noise floor learner & smooth suppressor
//! - [`DeEsser`]: Phase-coherent split-band 4.5–10 kHz sibilance detector & dynamic attenuator
//! - [`SpeechLeveler`]: Dual-rate speech loudness leveler ("Even Out") with noise-floor freeze
//! - [`VoiceDucker`]: Sidechain-driven music/bed track ducker with attack/hold/release
//! - [`VoiceCleanupChain`]: Unified 5-stage processor with `PodcastClean`, `BroadcastVoice`, and `NoisyRoom` presets

use crate::{
    dsp_math::{abs_f32, exp_f32, log10_f32, powf_f32, round_f32, sqrt_f32},
    Frame, RATE,
};

const PI_F64: f64 = 3.14159265358979323846;
const TAU_F64: f64 = 6.28318530717958647692;
const FRAC_PI_2_F64: f64 = 1.57079632679489661923;

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
fn sanitize_f32(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

#[inline]
pub(crate) fn frame_to_f32(frame: Frame) -> [f32; 2] {
    [frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]
}

#[inline]
pub(crate) fn f32_to_frame(sample: [f32; 2]) -> Frame {
    [
        if sample[0].is_finite() {
            round_f32(sample[0] * 32768.0).clamp(-32768.0, 32767.0) as i16
        } else {
            0
        },
        if sample[1].is_finite() {
            round_f32(sample[1] * 32768.0).clamp(-32768.0, 32767.0) as i16
        } else {
            0
        },
    ]
}

#[inline]
fn db_to_lin(db: f32) -> f32 {
    if !db.is_finite() || db <= -120.0 {
        return 0.0;
    }
    powf_f32(10.0, db / 20.0)
}

#[inline]
fn lin_to_db(lin: f32) -> f32 {
    let a = abs_f32(lin);
    if !a.is_finite() || a <= 1e-6 {
        return -120.0;
    }
    20.0 * log10_f32(a)
}

#[inline]
fn time_coeff(ms: f32, sample_rate: f32) -> f32 {
    let denom = ms.max(0.05) * 0.001 * sample_rate.max(1.0);
    exp_f32(-1.0 / denom)
}

/// Stereo Direct Form II Transposed biquad in `f64` precision for stable sub-100 Hz high-Q notches.
#[derive(Clone, Debug)]
struct StereoBiquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; 2],
    z2: [f64; 2],
}

impl StereoBiquad {
    fn bypass() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    fn notch(freq_hz: f32, q: f32, sample_rate: f32) -> Self {
        let sr = (sample_rate as f64).max(1000.0);
        let fc = (freq_hz as f64).clamp(5.0, sr * 0.45);
        let q = (q as f64).clamp(0.5, 100.0);
        let w0 = TAU_F64 * fc / sr;
        let cos_w0 = cos_f64(w0);
        let alpha = sin_f64(w0) / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: 1.0 / a0,
            b1: (-2.0 * cos_w0) / a0,
            b2: 1.0 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    fn lowpass(freq_hz: f32, q: f32, sample_rate: f32) -> Self {
        let sr = (sample_rate as f64).max(1000.0);
        let fc = (freq_hz as f64).clamp(5.0, sr * 0.45);
        let q = (q as f64).clamp(0.25, 20.0);
        let w0 = TAU_F64 * fc / sr;
        let cos_w0 = cos_f64(w0);
        let alpha = sin_f64(w0) / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b1 = 1.0 - cos_w0;
        Self {
            b0: (0.5 * b1) / a0,
            b1: b1 / a0,
            b2: (0.5 * b1) / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    fn highpass(freq_hz: f32, q: f32, sample_rate: f32) -> Self {
        let sr = (sample_rate as f64).max(1000.0);
        let fc = (freq_hz as f64).clamp(5.0, sr * 0.45);
        let q = (q as f64).clamp(0.25, 20.0);
        let w0 = TAU_F64 * fc / sr;
        let cos_w0 = cos_f64(w0);
        let alpha = sin_f64(w0) / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b0 = 0.5 * (1.0 + cos_w0);
        Self {
            b0: b0 / a0,
            b1: -(1.0 + cos_w0) / a0,
            b2: b0 / a0,
            a1: (-2.0 * cos_w0) / a0,
            a2: (1.0 - alpha) / a0,
            z1: [0.0; 2],
            z2: [0.0; 2],
        }
    }

    #[inline]
    fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for c in 0..2 {
            let x = sanitize_f32(input[c]) as f64;
            let y = self.b0 * x + self.z1[c];
            self.z1[c] = self.b1 * x - self.a1 * y + self.z2[c];
            self.z2[c] = self.b2 * x - self.a2 * y;
            out[c] = y as f32;
        }
        out
    }

    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
}

/// First-order all-pass stereo filter matching a Linkwitz-Riley 2nd-order crossover's `LP - HP` phase.
#[derive(Clone, Debug)]
struct StereoAllpass1 {
    coeff: f64,
    x1: [f64; 2],
    y1: [f64; 2],
}

impl StereoAllpass1 {
    fn new(freq_hz: f32, sample_rate: f32) -> Self {
        let sr = (sample_rate as f64).max(1000.0);
        let fc = (freq_hz as f64).clamp(5.0, sr * 0.45);
        let k = tan_f64(PI_F64 * fc / sr);
        let coeff = (k - 1.0) / (k + 1.0);
        Self {
            coeff,
            x1: [0.0; 2],
            y1: [0.0; 2],
        }
    }

    #[inline]
    fn process(&mut self, input: [f32; 2]) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for c in 0..2 {
            let x = sanitize_f32(input[c]) as f64;
            let y = self.coeff * x + self.x1[c] - self.coeff * self.y1[c];
            self.x1[c] = x;
            self.y1[c] = y;
            out[c] = y as f32;
        }
        out
    }

    fn reset(&mut self) {
        self.x1 = [0.0; 2];
        self.y1 = [0.0; 2];
    }
}

/// Phase-aligned 2-way Linkwitz-Riley 2nd-order crossover (`LP` + inverted `HP` = 1st-order allpass).
#[derive(Clone, Debug)]
struct LinkwitzRiley2 {
    lp: StereoBiquad,
    hp: StereoBiquad,
}

impl LinkwitzRiley2 {
    fn new(freq_hz: f32, sample_rate: f32) -> Self {
        Self {
            lp: StereoBiquad::lowpass(freq_hz, 0.5, sample_rate),
            hp: StereoBiquad::highpass(freq_hz, 0.5, sample_rate),
        }
    }

    #[inline]
    fn split(&mut self, input: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let low = self.lp.process(input);
        let high = self.hp.process(input);
        // Negating high yields exact 1st-order allpass reconstruction: low + (-high).
        (low, [-high[0], -high[1]])
    }

    fn reset(&mut self) {
        self.lp.reset();
        self.hp.reset();
    }
}

/// Mains hum fundamental frequency selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HumFrequency {
    /// 50 Hz mains (Europe, UK, Australia, most of Asia/Africa).
    Hz50,
    /// 60 Hz mains (North America, parts of South America/Japan).
    Hz60,
    /// Custom hum fundamental in Hz (clamped to `20.0..=120.0`).
    Custom(f32),
}

impl HumFrequency {
    pub fn hz(self) -> f32 {
        match self {
            Self::Hz50 => 50.0,
            Self::Hz60 => 60.0,
            Self::Custom(f) => if f.is_finite() { f.clamp(20.0, 120.0) } else { 60.0 },
        }
    }
}

/// Removes AC mains hum (50 Hz / 60 Hz) and up to its 6th harmonic using high-Q biquad notches.
#[derive(Clone, Debug)]
pub struct HumRemover {
    freq: HumFrequency,
    harmonics: u8,
    q: f32,
    sample_rate: f32,
    notches: [StereoBiquad; 6],
}

impl HumRemover {
    /// Creates a 6-harmonic high-Q (`Q = 25.0`) hum remover at 48 kHz.
    pub fn new(freq: HumFrequency) -> Self {
        Self::with_config(freq, 6, 25.0, RATE as f32)
    }

    /// Creates a customized hum remover (`harmonics` clamped to `1..=6`, `q` clamped to `2.0..=60.0`).
    pub fn with_config(freq: HumFrequency, harmonics: u8, q: f32, sample_rate: f32) -> Self {
        let harmonics = harmonics.clamp(1, 6);
        let q = if q.is_finite() { q.clamp(2.0, 60.0) } else { 25.0 };
        let sr = if sample_rate.is_finite() { sample_rate.clamp(8000.0, 192_000.0) } else { RATE as f32 };
        let base_hz = freq.hz();
        let notches = core::array::from_fn(|i| {
            if (i as u8) < harmonics {
                let harmonic_hz = base_hz * (i as f32 + 1.0);
                if harmonic_hz < sr * 0.45 {
                    StereoBiquad::notch(harmonic_hz, q, sr)
                } else {
                    StereoBiquad::bypass()
                }
            } else {
                StereoBiquad::bypass()
            }
        });
        Self {
            freq,
            harmonics,
            q,
            sample_rate: sr,
            notches,
        }
    }

    pub fn frequency(&self) -> HumFrequency {
        self.freq
    }

    pub fn harmonics(&self) -> u8 {
        self.harmonics
    }

    pub fn q(&self) -> f32 {
        self.q
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn reset(&mut self) {
        for n in &mut self.notches {
            n.reset();
        }
    }

    #[inline]
    pub fn process_frame_f32(&mut self, mut frame: [f32; 2]) -> [f32; 2] {
        for i in 0..self.harmonics as usize {
            frame = self.notches[i].process(frame);
        }
        frame
    }

    pub fn process_f32(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames {
            *f = self.process_frame_f32(*f);
        }
    }

    pub fn process_frames(&mut self, frames: &mut [Frame]) {
        for f in frames {
            let out = self.process_frame_f32(frame_to_f32(*f));
            *f = f32_to_frame(out);
        }
    }
}

/// 8-band spectral-envelope noise floor learner & smooth suppression gate.
/// Uses an all-pass complementary Linkwitz-Riley binary tree (`<180`, `180-400`, `400-800`,
/// `800-1500`, `1500-2800`, `2800-5000`, `5000-8500`, `>8500 Hz`) to suppress stationary hiss
/// and room noise without FFT musical-noise ringing.
#[derive(Clone, Debug)]
pub struct VoiceDenoise {
    sample_rate: f32,
    reduction_db: f32,
    min_gain: f32,
    sensitivity: f32,
    // Binary crossover tree: level 1, level 2 (x2), level 3 (x4)
    split_1500: LinkwitzRiley2,
    split_400: LinkwitzRiley2,
    split_5000: LinkwitzRiley2,
    split_180: LinkwitzRiley2,
    split_800: LinkwitzRiley2,
    split_2800: LinkwitzRiley2,
    split_8500: LinkwitzRiley2,
    // All-pass phase alignment across parallel branches so sum B0..B7 is 0 dB flat
    ap_l_5000: StereoAllpass1,
    ap_l_2800: StereoAllpass1,
    ap_l_8500: StereoAllpass1,
    ap_h_400: StereoAllpass1,
    ap_h_180: StereoAllpass1,
    ap_h_800: StereoAllpass1,
    ap_ll_800: StereoAllpass1,
    ap_lh_180: StereoAllpass1,
    ap_hl_8500: StereoAllpass1,
    ap_hh_2800: StereoAllpass1,
    // Per-band envelope and adaptive noise floor
    band_env: [f32; 8],
    noise_floor: [f32; 8],
    band_gain: [f32; 8],
    locked_profile: bool,
    calibrated_frames: u64,
    warm_up_frames: u64,
    // Smoothing coefficients
    env_attack: f32,
    env_release: f32,
    floor_down: f32,
    floor_up: f32,
    gain_attack: f32,
    gain_release: f32,
}

impl VoiceDenoise {
    /// Creates a studio voice denoiser with 16 dB max reduction at 48 kHz.
    pub fn new() -> Self {
        Self::with_config(16.0, 1.6, RATE as f32)
    }

    /// Creates a denoiser with `reduction_db` (`0.0..=36.0`) and `sensitivity` (`0.8..=4.0`).
    pub fn with_config(reduction_db: f32, sensitivity: f32, sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() { sample_rate.clamp(8000.0, 192_000.0) } else { RATE as f32 };
        let red = if reduction_db.is_finite() { reduction_db.clamp(0.0, 36.0) } else { 16.0 };
        let sens = if sensitivity.is_finite() { sensitivity.clamp(0.8, 4.0) } else { 1.6 };
        Self {
            sample_rate: sr,
            reduction_db: red,
            min_gain: db_to_lin(-red),
            sensitivity: sens,
            split_1500: LinkwitzRiley2::new(1500.0, sr),
            split_400: LinkwitzRiley2::new(400.0, sr),
            split_5000: LinkwitzRiley2::new(5000.0, sr),
            split_180: LinkwitzRiley2::new(180.0, sr),
            split_800: LinkwitzRiley2::new(800.0, sr),
            split_2800: LinkwitzRiley2::new(2800.0, sr),
            split_8500: LinkwitzRiley2::new(8500.0, sr),
            ap_l_5000: StereoAllpass1::new(5000.0, sr),
            ap_l_2800: StereoAllpass1::new(2800.0, sr),
            ap_l_8500: StereoAllpass1::new(8500.0, sr),
            ap_h_400: StereoAllpass1::new(400.0, sr),
            ap_h_180: StereoAllpass1::new(180.0, sr),
            ap_h_800: StereoAllpass1::new(800.0, sr),
            ap_ll_800: StereoAllpass1::new(800.0, sr),
            ap_lh_180: StereoAllpass1::new(180.0, sr),
            ap_hl_8500: StereoAllpass1::new(8500.0, sr),
            ap_hh_2800: StereoAllpass1::new(2800.0, sr),
            band_env: [0.0; 8],
            noise_floor: [1e-4; 8],
            band_gain: [1.0; 8],
            locked_profile: false,
            calibrated_frames: 0,
            warm_up_frames: (sr * 0.015) as u64,
            env_attack: time_coeff(4.0, sr),
            env_release: time_coeff(55.0, sr),
            floor_down: time_coeff(35.0, sr),
            floor_up: time_coeff(2500.0, sr),
            gain_attack: time_coeff(8.0, sr),
            gain_release: time_coeff(85.0, sr),
        }
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn reduction_db(&self) -> f32 {
        self.reduction_db
    }

    pub fn noise_floor_bands(&self) -> [f32; 8] {
        self.noise_floor
    }

    pub fn reset(&mut self) {
        self.split_1500.reset();
        self.split_400.reset();
        self.split_5000.reset();
        self.split_180.reset();
        self.split_800.reset();
        self.split_2800.reset();
        self.split_8500.reset();
        self.ap_l_5000.reset();
        self.ap_l_2800.reset();
        self.ap_l_8500.reset();
        self.ap_h_400.reset();
        self.ap_h_180.reset();
        self.ap_h_800.reset();
        self.ap_ll_800.reset();
        self.ap_lh_180.reset();
        self.ap_hl_8500.reset();
        self.ap_hh_2800.reset();
        self.band_env = [0.0; 8];
        self.band_gain = [1.0; 8];
        if !self.locked_profile {
            self.noise_floor = [1e-4; 8];
            self.calibrated_frames = 0;
        }
    }

    pub fn reset_noise_profile(&mut self) {
        self.locked_profile = false;
        self.noise_floor = [1e-4; 8];
        self.calibrated_frames = 0;
    }

    #[inline]
    fn decompose(&mut self, input: [f32; 2]) -> [[f32; 2]; 8] {
        let (l, h) = self.split_1500.split(input);
        let l = self.ap_l_8500.process(self.ap_l_2800.process(self.ap_l_5000.process(l)));
        let h = self.ap_h_800.process(self.ap_h_180.process(self.ap_h_400.process(h)));
        let (ll, lh) = self.split_400.split(l);
        let (hl, hh) = self.split_5000.split(h);
        let ll = self.ap_ll_800.process(ll);
        let lh = self.ap_lh_180.process(lh);
        let hl = self.ap_hl_8500.process(hl);
        let hh = self.ap_hh_2800.process(hh);
        let (b0, b1) = self.split_180.split(ll);
        let (b2, b3) = self.split_800.split(lh);
        let (b4, b5) = self.split_2800.split(hl);
        let (b6, b7) = self.split_8500.split(hh);
        [b0, b1, b2, b3, b4, b5, b6, b7]
    }

    fn reset_filters(&mut self) {
        self.split_1500.reset();
        self.split_400.reset();
        self.split_5000.reset();
        self.split_180.reset();
        self.split_800.reset();
        self.split_2800.reset();
        self.split_8500.reset();
        self.ap_l_5000.reset();
        self.ap_l_2800.reset();
        self.ap_l_8500.reset();
        self.ap_h_400.reset();
        self.ap_h_180.reset();
        self.ap_h_800.reset();
        self.ap_ll_800.reset();
        self.ap_lh_180.reset();
        self.ap_hl_8500.reset();
        self.ap_hh_2800.reset();
    }

    /// Explicitly learns a noise profile from a room-tone snippet and locks it against speech drift.
    pub fn learn_noise_f32(&mut self, noise_frames: &[[f32; 2]]) {
        if noise_frames.is_empty() {
            return;
        }
        self.reset_filters();

        let mut sum_sq = [0.0f64; 8];
        for &frame in noise_frames {
            let bands = self.decompose(frame);
            for (b, sample) in bands.iter().enumerate() {
                let energy = 0.5 * ((sample[0] as f64) * (sample[0] as f64)
                    + (sample[1] as f64) * (sample[1] as f64));
                sum_sq[b] += energy;
            }
        }
        let n = noise_frames.len() as f64;
        for b in 0..8 {
            let rms = sqrt_f32((sum_sq[b] / n) as f32).max(1e-6);
            self.noise_floor[b] = rms;
            self.band_env[b] = rms;
        }
        self.locked_profile = true;
        self.calibrated_frames = noise_frames.len() as u64;
    }

    pub fn learn_noise_frames(&mut self, noise_frames: &[Frame]) {
        if noise_frames.is_empty() {
            return;
        }
        self.reset_filters();

        let mut sum_sq = [0.0f64; 8];
        for &frame in noise_frames {
            let f = frame_to_f32(frame);
            let bands = self.decompose(f);
            for (b, sample) in bands.iter().enumerate() {
                let energy = 0.5 * ((sample[0] as f64) * (sample[0] as f64)
                    + (sample[1] as f64) * (sample[1] as f64));
                sum_sq[b] += energy;
            }
        }
        let n = noise_frames.len() as f64;
        for b in 0..8 {
            let rms = sqrt_f32((sum_sq[b] / n) as f32).max(1e-6);
            self.noise_floor[b] = rms;
            self.band_env[b] = rms;
        }
        self.locked_profile = true;
        self.calibrated_frames = noise_frames.len() as u64;
    }

    #[inline]
    pub fn process_frame_f32(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let bands = self.decompose(frame);
        self.calibrated_frames = self.calibrated_frames.saturating_add(1);
        let init_phase = !self.locked_profile && self.calibrated_frames <= self.warm_up_frames;

        let mut out = [0.0f32; 2];
        for b in 0..8 {
            let level = sqrt_f32(0.5 * (bands[b][0] * bands[b][0] + bands[b][1] * bands[b][1]));
            let ec = if level > self.band_env[b] {
                self.env_attack
            } else {
                self.env_release
            };
            self.band_env[b] = ec * self.band_env[b] + (1.0 - ec) * level;

            if !self.locked_profile {
                if self.calibrated_frames == self.warm_up_frames / 2 + 1 {
                    self.noise_floor[b] = self.band_env[b].max(1e-6);
                } else if !init_phase {
                    let fc = if self.band_env[b] < self.noise_floor[b] {
                        self.floor_down
                    } else {
                        self.floor_up
                    };
                    self.noise_floor[b] = (fc * self.noise_floor[b] + (1.0 - fc) * self.band_env[b]).max(1e-6);
                }
            }

            let target_gain = if init_phase || self.reduction_db <= 0.0 {
                1.0
            } else {
                let env_sq = self.band_env[b] * self.band_env[b];
                let thresh = self.noise_floor[b] * self.sensitivity;
                let thresh_sq = thresh * thresh;
                if env_sq <= 1e-12 {
                    self.min_gain
                } else {
                    // Smooth Wiener-style spectral subtraction gate.
                    let snr_excess = (env_sq - thresh_sq).max(0.0) / (env_sq + 1e-12);
                    let wiener = sqrt_f32(snr_excess);
                    self.min_gain + (1.0 - self.min_gain) * wiener
                }
            };

            let gc = if target_gain > self.band_gain[b] {
                self.gain_attack
            } else {
                self.gain_release
            };
            self.band_gain[b] = gc * self.band_gain[b] + (1.0 - gc) * target_gain;

            out[0] += bands[b][0] * self.band_gain[b];
            out[1] += bands[b][1] * self.band_gain[b];
        }
        out
    }

    pub fn process_f32(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames {
            *f = self.process_frame_f32(*f);
        }
    }

    pub fn process_frames(&mut self, frames: &mut [Frame]) {
        for f in frames {
            let out = self.process_frame_f32(frame_to_f32(*f));
            *f = f32_to_frame(out);
        }
    }
}

impl Default for VoiceDenoise {
    fn default() -> Self {
        Self::new()
    }
}

/// 4th-order Linkwitz-Riley (24 dB/oct) crossover (`LP4 + HP4` has exact 0.0 dB flat allpass sum).
#[derive(Clone, Debug)]
struct LinkwitzRiley4 {
    lp1: StereoBiquad,
    lp2: StereoBiquad,
    hp1: StereoBiquad,
    hp2: StereoBiquad,
}

impl LinkwitzRiley4 {
    fn new(freq_hz: f32, sample_rate: f32) -> Self {
        let q = 0.70710678;
        Self {
            lp1: StereoBiquad::lowpass(freq_hz, q, sample_rate),
            lp2: StereoBiquad::lowpass(freq_hz, q, sample_rate),
            hp1: StereoBiquad::highpass(freq_hz, q, sample_rate),
            hp2: StereoBiquad::highpass(freq_hz, q, sample_rate),
        }
    }

    #[inline]
    fn split(&mut self, input: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let low = self.lp2.process(self.lp1.process(input));
        let high = self.hp2.process(self.hp1.process(input));
        (low, high)
    }

    fn reset(&mut self) {
        self.lp1.reset();
        self.lp2.reset();
        self.hp1.reset();
        self.hp2.reset();
    }
}

/// Split-band `4.5–10 kHz` sibilance detector & dynamic attenuator.
/// Uses a 24 dB/oct Linkwitz-Riley 4th-order crossover at 4.5 kHz driven by a dedicated
/// `4.5–10 kHz` sibilance sidechain so harsh `"s"`/`"sh"` transients are attenuated cleanly
/// without dulling vocal fundamentals.
#[derive(Clone, Debug)]
pub struct DeEsser {
    threshold_db: f32,
    threshold_lin: f32,
    ratio: f32,
    max_reduction_db: f32,
    split_4k5: LinkwitzRiley4,
    sc_lp_10k: StereoBiquad,
    env: f32,
    attack_coeff: f32,
    release_coeff: f32,
    last_gain: f32,
}

impl DeEsser {
    /// Creates a studio voice de-esser (`-24 dBFS` threshold, `5.5:1` ratio, up to `12 dB` cut).
    pub fn new() -> Self {
        Self::with_config(-24.0, 5.5, 12.0, RATE as f32)
    }

    pub fn with_config(
        threshold_db: f32,
        ratio: f32,
        max_reduction_db: f32,
        sample_rate: f32,
    ) -> Self {
        let sr = if sample_rate.is_finite() { sample_rate.clamp(16_000.0, 192_000.0) } else { RATE as f32 };
        let thresh = if threshold_db.is_finite() { threshold_db.clamp(-60.0, 0.0) } else { -24.0 };
        let ratio = if ratio.is_finite() { ratio.clamp(1.0, 20.0) } else { 4.0 };
        let max_red = if max_reduction_db.is_finite() { max_reduction_db.clamp(0.0, 30.0) } else { 12.0 };
        Self {
            threshold_db: thresh,
            threshold_lin: db_to_lin(thresh),
            ratio,
            max_reduction_db: max_red,
            split_4k5: LinkwitzRiley4::new(4500.0, sr),
            sc_lp_10k: StereoBiquad::lowpass(10_000.0_f32.min(sr * 0.42), 0.70710678, sr),
            env: 0.0,
            attack_coeff: time_coeff(0.8, sr),
            release_coeff: time_coeff(45.0, sr),
            last_gain: 1.0,
        }
    }

    pub fn last_gain_reduction_db(&self) -> f32 {
        -lin_to_db(self.last_gain).min(0.0)
    }

    pub fn reset(&mut self) {
        self.split_4k5.reset();
        self.sc_lp_10k.reset();
        self.env = 0.0;
        self.last_gain = 1.0;
    }

    #[inline]
    pub fn process_frame_f32(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let (low, high) = self.split_4k5.split(frame);
        let sib_band = self.sc_lp_10k.process(high);

        let sib_peak = abs_f32(sib_band[0]).max(abs_f32(sib_band[1]));
        let c = if sib_peak > self.env {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.env = c * self.env + (1.0 - c) * sib_peak;

        let gain = if self.env > self.threshold_lin && self.ratio > 1.0 && self.max_reduction_db > 0.0 {
            let env_db = lin_to_db(self.env);
            let over_db = (env_db - self.threshold_db).max(0.0);
            let red_db = (over_db * (1.0 - 1.0 / self.ratio)).min(self.max_reduction_db);
            db_to_lin(-red_db)
        } else {
            1.0
        };
        self.last_gain = gain;

        [
            low[0] + high[0] * gain,
            low[1] + high[1] * gain,
        ]
    }

    pub fn process_f32(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames {
            *f = self.process_frame_f32(*f);
        }
    }

    pub fn process_frames(&mut self, frames: &mut [Frame]) {
        for f in frames {
            let out = self.process_frame_f32(frame_to_f32(*f));
            *f = f32_to_frame(out);
        }
    }
}

impl Default for DeEsser {
    fn default() -> Self {
        Self::new()
    }
}

/// OmaAudio "Even Out" speech leveler: brings quiet and loud speech phrases to an even target
/// RMS level while freezing gain during silence/background noise and limiting peaks below `-1 dBFS`.
#[derive(Clone, Debug)]
pub struct SpeechLeveler {
    target_db: f32,
    gate_db: f32,
    max_boost_db: f32,
    max_cut_db: f32,
    rms_sq: f32,
    rms_coeff: f32,
    gain_lin: f32,
    gain_attack: f32,
    gain_release: f32,
    peak_ceiling: f32,
}

impl SpeechLeveler {
    /// Creates a speech leveler targeting `-16.0 dBFS` RMS with `-48.0 dBFS` noise gate.
    pub fn new() -> Self {
        Self::with_config(-16.0, -48.0, 12.0, 18.0, RATE as f32)
    }

    pub fn with_config(
        target_db: f32,
        gate_db: f32,
        max_boost_db: f32,
        max_cut_db: f32,
        sample_rate: f32,
    ) -> Self {
        let sr = if sample_rate.is_finite() { sample_rate.clamp(8000.0, 192_000.0) } else { RATE as f32 };
        let target = if target_db.is_finite() { target_db.clamp(-40.0, -3.0) } else { -16.0 };
        let gate = if gate_db.is_finite() { gate_db.clamp(-80.0, -20.0) } else { -48.0 };
        let boost = if max_boost_db.is_finite() { max_boost_db.clamp(0.0, 24.0) } else { 12.0 };
        let cut = if max_cut_db.is_finite() { max_cut_db.clamp(0.0, 30.0) } else { 18.0 };
        Self {
            target_db: target,
            gate_db: gate,
            max_boost_db: boost,
            max_cut_db: cut,
            rms_sq: 0.0,
            rms_coeff: time_coeff(120.0, sr),
            gain_lin: 1.0,
            gain_attack: time_coeff(18.0, sr),
            gain_release: time_coeff(220.0, sr),
            peak_ceiling: db_to_lin(-1.0),
        }
    }

    pub fn current_gain_db(&self) -> f32 {
        lin_to_db(self.gain_lin)
    }

    pub fn reset(&mut self) {
        self.rms_sq = 0.0;
        self.gain_lin = 1.0;
    }

    #[inline]
    pub fn process_frame_f32(&mut self, frame: [f32; 2]) -> [f32; 2] {
        let x0 = sanitize_f32(frame[0]);
        let x1 = sanitize_f32(frame[1]);
        let energy = 0.5 * (x0 * x0 + x1 * x1);
        self.rms_sq = self.rms_coeff * self.rms_sq + (1.0 - self.rms_coeff) * energy;
        let rms = sqrt_f32(self.rms_sq);
        let rms_db = lin_to_db(rms);

        let desired_gain = if rms_db < self.gate_db {
            // Below speech gate: do not amplify room noise; smoothly return toward unity.
            1.0
        } else {
            let delta_db = (self.target_db - rms_db).clamp(-self.max_cut_db, self.max_boost_db);
            // Smooth 0.85 compression slope toward target so micro-inflections remain natural.
            db_to_lin(delta_db * 0.9)
        };

        let gc = if desired_gain < self.gain_lin {
            self.gain_attack
        } else {
            self.gain_release
        };
        self.gain_lin = gc * self.gain_lin + (1.0 - gc) * desired_gain;

        let mut y0 = x0 * self.gain_lin;
        let mut y1 = x1 * self.gain_lin;
        let peak = abs_f32(y0).max(abs_f32(y1));
        if peak > self.peak_ceiling {
            let scale = self.peak_ceiling / peak;
            y0 *= scale;
            y1 *= scale;
        }
        [y0, y1]
    }

    pub fn process_f32(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames {
            *f = self.process_frame_f32(*f);
        }
    }

    pub fn process_frames(&mut self, frames: &mut [Frame]) {
        for f in frames {
            let out = self.process_frame_f32(frame_to_f32(*f));
            *f = f32_to_frame(out);
        }
    }
}

impl Default for SpeechLeveler {
    fn default() -> Self {
        Self::new()
    }
}

/// Sidechain voice ducker: automatically attenuates a background music/bed track when speech is active,
/// using configurable threshold, duck depth, attack, hold, and release.
#[derive(Clone, Debug)]
pub struct VoiceDucker {
    threshold_lin: f32,
    duck_gain: f32,
    attack_coeff: f32,
    release_coeff: f32,
    hold_frames: u32,
    hold_remaining: u32,
    sidechain_env: f32,
    sc_attack: f32,
    sc_release: f32,
    current_gain: f32,
}

impl VoiceDucker {
    /// Creates a podcast ducker (`-32 dBFS` speech trigger, `-12 dB` duck, 15 ms attack, 250 ms hold, 350 ms release).
    pub fn new() -> Self {
        Self::with_config(-32.0, -12.0, 15.0, 250.0, 350.0, RATE as f32)
    }

    pub fn with_config(
        threshold_db: f32,
        duck_db: f32,
        attack_ms: f32,
        hold_ms: f32,
        release_ms: f32,
        sample_rate: f32,
    ) -> Self {
        let sr = if sample_rate.is_finite() { sample_rate.clamp(8000.0, 192_000.0) } else { RATE as f32 };
        let thresh = if threshold_db.is_finite() { threshold_db.clamp(-80.0, 0.0) } else { -32.0 };
        let duck = if duck_db.is_finite() { duck_db.clamp(-40.0, 0.0) } else { -12.0 };
        let hold_f = ((hold_ms.max(0.0) * 0.001) * sr) as u32;
        Self {
            threshold_lin: db_to_lin(thresh),
            duck_gain: db_to_lin(duck),
            attack_coeff: time_coeff(attack_ms, sr),
            release_coeff: time_coeff(release_ms, sr),
            hold_frames: hold_f,
            hold_remaining: 0,
            sidechain_env: 0.0,
            sc_attack: time_coeff(3.0, sr),
            sc_release: time_coeff(40.0, sr),
            current_gain: 1.0,
        }
    }

    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }

    pub fn current_gain_db(&self) -> f32 {
        lin_to_db(self.current_gain)
    }

    pub fn reset(&mut self) {
        self.hold_remaining = 0;
        self.sidechain_env = 0.0;
        self.current_gain = 1.0;
    }

    /// Advances the ducker sidechain on `speech` and returns the attenuated `bed` frame.
    #[inline]
    pub fn process_frame_f32(&mut self, speech: [f32; 2], bed: [f32; 2]) -> [f32; 2] {
        let sc = abs_f32(sanitize_f32(speech[0])).max(abs_f32(sanitize_f32(speech[1])));
        let sc_c = if sc > self.sidechain_env {
            self.sc_attack
        } else {
            self.sc_release
        };
        self.sidechain_env = sc_c * self.sidechain_env + (1.0 - sc_c) * sc;

        if self.sidechain_env >= self.threshold_lin {
            self.hold_remaining = self.hold_frames;
        } else if self.hold_remaining > 0 {
            self.hold_remaining -= 1;
        }

        let target = if self.hold_remaining > 0 || self.sidechain_env >= self.threshold_lin {
            self.duck_gain
        } else {
            1.0
        };

        let c = if target < self.current_gain {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.current_gain = c * self.current_gain + (1.0 - c) * target;

        [
            sanitize_f32(bed[0]) * self.current_gain,
            sanitize_f32(bed[1]) * self.current_gain,
        ]
    }

    pub fn process_bed_f32(&mut self, speech: &[[f32; 2]], bed: &mut [[f32; 2]]) {
        let n = speech.len().min(bed.len());
        for i in 0..n {
            bed[i] = self.process_frame_f32(speech[i], bed[i]);
        }
    }

    pub fn mix_f32(&mut self, speech: &[[f32; 2]], bed: &[[f32; 2]], out: &mut [[f32; 2]]) {
        let n = speech.len().min(bed.len()).min(out.len());
        for i in 0..n {
            let ducked = self.process_frame_f32(speech[i], bed[i]);
            out[i] = [
                sanitize_f32(speech[i][0]) + ducked[0],
                sanitize_f32(speech[i][1]) + ducked[1],
            ];
        }
    }

    pub fn process_bed_frames(&mut self, speech: &[Frame], bed: &mut [Frame]) {
        let n = speech.len().min(bed.len());
        for i in 0..n {
            let ducked = self.process_frame_f32(frame_to_f32(speech[i]), frame_to_f32(bed[i]));
            bed[i] = f32_to_frame(ducked);
        }
    }

    pub fn mix_frames(&mut self, speech: &[Frame], bed: &[Frame], out: &mut [Frame]) {
        let n = speech.len().min(bed.len()).min(out.len());
        for i in 0..n {
            let sp = frame_to_f32(speech[i]);
            let ducked = self.process_frame_f32(sp, frame_to_f32(bed[i]));
            out[i] = f32_to_frame([sp[0] + ducked[0], sp[1] + ducked[1]]);
        }
    }
}

impl Default for VoiceDucker {
    fn default() -> Self {
        Self::new()
    }
}

/// Curated Quick Voice Fixes presets matching OmaAudio studio templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoicePreset {
    /// Clean, warm podcast speech: 60 Hz hum notch, gentle denoise, balanced de-esser, -16 dBFS leveler.
    PodcastClean,
    /// Broadcast / radio presence: 60 Hz hum notch + 80 Hz rumble filter, crisp de-esser, -14 dBFS leveler.
    BroadcastVoice,
    /// Aggressive cleanup for untreated / noisy rooms: strong denoise, 6-harmonic hum notch, controlled leveler.
    NoisyRoom,
}

/// Unified Quick Voice Fixes processor combining:
/// 1. [`HumRemover`]
/// 2. [`VoiceDenoise`]
/// 3. [`DeEsser`]
/// 4. [`SpeechLeveler`]
/// 5. [`VoiceDucker`] (for mixing speech over a music/bed track)
#[derive(Clone, Debug)]
pub struct VoiceCleanupChain {
    pub hum_enabled: bool,
    pub denoise_enabled: bool,
    pub de_esser_enabled: bool,
    pub leveler_enabled: bool,
    pub hum: HumRemover,
    pub denoise: VoiceDenoise,
    pub de_esser: DeEsser,
    pub leveler: SpeechLeveler,
    pub ducker: VoiceDucker,
    highpass: Option<StereoBiquad>,
}

impl VoiceCleanupChain {
    pub fn from_preset(preset: VoicePreset) -> Self {
        Self::from_preset_with_mains(preset, HumFrequency::Hz60)
    }

    pub fn from_preset_with_mains(preset: VoicePreset, mains: HumFrequency) -> Self {
        let sr = RATE as f32;
        match preset {
            VoicePreset::PodcastClean => Self {
                hum_enabled: true,
                denoise_enabled: true,
                de_esser_enabled: true,
                leveler_enabled: true,
                hum: HumRemover::with_config(mains, 6, 25.0, sr),
                denoise: VoiceDenoise::with_config(14.0, 1.5, sr),
                de_esser: DeEsser::with_config(-24.0, 4.0, 10.0, sr),
                leveler: SpeechLeveler::with_config(-16.0, -48.0, 12.0, 16.0, sr),
                ducker: VoiceDucker::with_config(-32.0, -12.0, 15.0, 250.0, 350.0, sr),
                highpass: None,
            },
            VoicePreset::BroadcastVoice => Self {
                hum_enabled: true,
                denoise_enabled: true,
                de_esser_enabled: true,
                leveler_enabled: true,
                hum: HumRemover::with_config(mains, 6, 28.0, sr),
                denoise: VoiceDenoise::with_config(16.0, 1.6, sr),
                de_esser: DeEsser::with_config(-22.0, 5.0, 12.0, sr),
                leveler: SpeechLeveler::with_config(-14.0, -46.0, 14.0, 18.0, sr),
                ducker: VoiceDucker::with_config(-30.0, -14.0, 10.0, 220.0, 300.0, sr),
                highpass: Some(StereoBiquad::highpass(80.0, 0.707, sr)),
            },
            VoicePreset::NoisyRoom => Self {
                hum_enabled: true,
                denoise_enabled: true,
                de_esser_enabled: true,
                leveler_enabled: true,
                hum: HumRemover::with_config(mains, 6, 20.0, sr),
                denoise: VoiceDenoise::with_config(24.0, 2.0, sr),
                de_esser: DeEsser::with_config(-25.0, 4.5, 11.0, sr),
                leveler: SpeechLeveler::with_config(-16.0, -44.0, 9.0, 18.0, sr),
                ducker: VoiceDucker::with_config(-30.0, -15.0, 12.0, 280.0, 400.0, sr),
                highpass: Some(StereoBiquad::highpass(90.0, 0.707, sr)),
            },
        }
    }

    pub fn new() -> Self {
        Self::from_preset(VoicePreset::PodcastClean)
    }

    pub fn reset(&mut self) {
        self.hum.reset();
        self.denoise.reset();
        self.de_esser.reset();
        self.leveler.reset();
        self.ducker.reset();
        if let Some(hp) = &mut self.highpass {
            hp.reset();
        }
    }

    #[inline]
    pub fn process_frame_f32(&mut self, mut frame: [f32; 2]) -> [f32; 2] {
        if let Some(hp) = &mut self.highpass {
            frame = hp.process(frame);
        }
        if self.hum_enabled {
            frame = self.hum.process_frame_f32(frame);
        }
        if self.denoise_enabled {
            frame = self.denoise.process_frame_f32(frame);
        }
        if self.de_esser_enabled {
            frame = self.de_esser.process_frame_f32(frame);
        }
        if self.leveler_enabled {
            frame = self.leveler.process_frame_f32(frame);
        }
        frame
    }

    pub fn process_f32(&mut self, frames: &mut [[f32; 2]]) {
        for f in frames {
            *f = self.process_frame_f32(*f);
        }
    }

    pub fn process_frames(&mut self, frames: &mut [Frame]) {
        for f in frames {
            let out = self.process_frame_f32(frame_to_f32(*f));
            *f = f32_to_frame(out);
        }
    }

    /// Cleans `voice_and_out` in place and mixes the automatically ducked `bed` track underneath it.
    pub fn process_with_bed_f32(&mut self, voice_and_out: &mut [[f32; 2]], bed: &[[f32; 2]]) {
        let n = voice_and_out.len();
        for i in 0..n {
            let clean = self.process_frame_f32(voice_and_out[i]);
            let bed_sample = bed.get(i).copied().unwrap_or([0.0, 0.0]);
            let ducked = self.ducker.process_frame_f32(clean, bed_sample);
            voice_and_out[i] = [clean[0] + ducked[0], clean[1] + ducked[1]];
        }
    }

    pub fn process_with_bed_frames(&mut self, voice_and_out: &mut [Frame], bed: &[Frame]) {
        let n = voice_and_out.len();
        for i in 0..n {
            let clean = self.process_frame_f32(frame_to_f32(voice_and_out[i]));
            let bed_sample = bed.get(i).copied().map(frame_to_f32).unwrap_or([0.0, 0.0]);
            let ducked = self.ducker.process_frame_f32(clean, bed_sample);
            voice_and_out[i] = f32_to_frame([clean[0] + ducked[0], clean[1] + ducked[1]]);
        }
    }
}

impl Default for VoiceCleanupChain {
    fn default() -> Self {
        Self::new()
    }
}
