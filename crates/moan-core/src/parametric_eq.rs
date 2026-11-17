// SPDX-License-Identifier: GPL-3.0-only
//! 6-band RBJ biquad parametric EQ, exact cascaded complex magnitude response,
//! #![no_std] radix-2 stereo-power FFT spectrum analyzer, and narrow-peak resonance detector.

use crate::{
    dsp_math::{abs_f32, cos_f32, log10_f32, powf_f32, round_f32, sin_f32, sqrt_f32, PI, TAU},
    Frame, RATE,
};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Frequency,
    Gain,
    Q,
    Band,
    Size,
    Memory,
}

#[inline]
pub(crate) fn frame_to_f32(frame: Frame) -> [f32; 2] {
    [frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0]
}

#[inline]
pub(crate) fn f32_to_frame(lr: [f32; 2]) -> Frame {
    [sample_f32_to_i16(lr[0]), sample_f32_to_i16(lr[1])]
}

#[inline]
fn sample_f32_to_i16(v: f32) -> i16 {
    if !v.is_finite() {
        return 0;
    }
    round_f32(v * 32768.0).clamp(-32768.0, 32767.0) as i16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BandKind {
    HighPass,
    LowShelf,
    Bell,
    HighShelf,
    LowPass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandConfig {
    pub enabled: bool,
    pub freq_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// Shared RBJ biquad section with Direct Form II Transposed f64 state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; 2],
    z2: [f64; 2],
    bypass: bool,
}

impl Default for Biquad {
    fn default() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: [0.0; 2],
            z2: [0.0; 2],
            bypass: true,
        }
    }
}

impl Biquad {
    pub(crate) fn set_bypass(&mut self) {
        self.b0 = 1.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.a1 = 0.0;
        self.a2 = 0.0;
        self.bypass = true;
    }

    pub(crate) fn configure(&mut self, kind: BandKind, freq_hz: f32, gain_db: f32, q: f32) {
        if matches!(
            kind,
            BandKind::LowShelf | BandKind::Bell | BandKind::HighShelf
        ) && abs_f32(gain_db) < 1e-4
        {
            self.set_bypass();
            return;
        }

        let w0 = TAU * freq_hz / RATE as f32;
        let half_w0 = 0.5 * w0;
        let s_half = sin_f32(half_w0) as f64;
        let c_half = cos_f32(half_w0) as f64;
        let sin_w0 = 2.0 * s_half * c_half;
        let cos_w0 = 1.0 - 2.0 * s_half * s_half;
        let alpha = sin_w0 / (2.0 * q as f64);

        let (b0_u, b1_u, b2_u, a0_u, a1_u, a2_u) = match kind {
            BandKind::HighPass => {
                let c2 = c_half * c_half;
                (c2, -2.0 * c2, c2, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha)
            }
            BandKind::LowPass => {
                let s2 = s_half * s_half;
                (s2, 2.0 * s2, s2, 1.0 + alpha, -2.0 * cos_w0, 1.0 - alpha)
            }
            BandKind::Bell => {
                let a = powf_f32(10.0, gain_db / 40.0) as f64;
                (
                    1.0 + alpha * a,
                    -2.0 * cos_w0,
                    1.0 - alpha * a,
                    1.0 + alpha / a,
                    -2.0 * cos_w0,
                    1.0 - alpha / a,
                )
            }
            BandKind::LowShelf => {
                let a = powf_f32(10.0, gain_db / 40.0) as f64;
                let sqrt_a = sqrt_f32(a as f32) as f64;
                let two_sqrt_a_alpha = 2.0 * sqrt_a * alpha;
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                (
                    a * (ap1 - am1 * cos_w0 + two_sqrt_a_alpha),
                    2.0 * a * (am1 - ap1 * cos_w0),
                    a * (ap1 - am1 * cos_w0 - two_sqrt_a_alpha),
                    ap1 + am1 * cos_w0 + two_sqrt_a_alpha,
                    -2.0 * (am1 + ap1 * cos_w0),
                    ap1 + am1 * cos_w0 - two_sqrt_a_alpha,
                )
            }
            BandKind::HighShelf => {
                let a = powf_f32(10.0, gain_db / 40.0) as f64;
                let sqrt_a = sqrt_f32(a as f32) as f64;
                let two_sqrt_a_alpha = 2.0 * sqrt_a * alpha;
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                (
                    a * (ap1 + am1 * cos_w0 + two_sqrt_a_alpha),
                    -2.0 * a * (am1 + ap1 * cos_w0),
                    a * (ap1 + am1 * cos_w0 - two_sqrt_a_alpha),
                    ap1 - am1 * cos_w0 + two_sqrt_a_alpha,
                    2.0 * (am1 - ap1 * cos_w0),
                    ap1 - am1 * cos_w0 - two_sqrt_a_alpha,
                )
            }
        };

        let inv_a0 = 1.0 / a0_u;
        self.b0 = b0_u * inv_a0;
        self.b1 = b1_u * inv_a0;
        self.b2 = b2_u * inv_a0;
        self.a1 = a1_u * inv_a0;
        self.a2 = a2_u * inv_a0;
        self.bypass = false;
    }

    #[inline]
    pub(crate) fn process(&mut self, lr: [f32; 2]) -> [f32; 2] {
        if self.bypass {
            return lr;
        }
        let mut out = [0.0f32; 2];
        for c in 0..2 {
            let x = lr[c] as f64;
            let y = self.b0 * x + self.z1[c];
            self.z1[c] = self.b1 * x - self.a1 * y + self.z2[c];
            self.z2[c] = self.b2 * x - self.a2 * y;
            out[c] = y as f32;
        }
        out
    }

    pub(crate) fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }

    /// Exact squared magnitude |H(e^{j w})|^2 using half-angle phi = sin^2(w / 2).
    #[inline]
    pub(crate) fn magnitude_sq(&self, phi: f64) -> f64 {
        if self.bypass {
            return 1.0;
        }
        let num_sum = self.b0 + self.b1 + self.b2;
        let den_sum = 1.0 + self.a1 + self.a2;
        let num = num_sum * num_sum
            - 4.0 * (self.b0 * self.b1 + 4.0 * self.b0 * self.b2 + self.b1 * self.b2) * phi
            + 16.0 * self.b0 * self.b2 * phi * phi;
        let den = den_sum * den_sum
            - 4.0 * (self.a1 + 4.0 * self.a2 + self.a1 * self.a2) * phi
            + 16.0 * self.a2 * phi * phi;
        num.max(0.0) / den.max(1e-30)
    }
}

const BAND_KINDS: [BandKind; 6] = [
    BandKind::HighPass,
    BandKind::LowShelf,
    BandKind::Bell,
    BandKind::Bell,
    BandKind::HighShelf,
    BandKind::LowPass,
];

/// 6-band RBJ biquad parametric equalizer at 48 kHz:
/// - Band 0: High-pass (`20..=1000 Hz`, Q `0.5..=4.0`, enable bool)
/// - Band 1: Low Shelf (`30..=500 Hz`, `-18..=+18 dB`, Q `0.5..=2.0`)
/// - Band 2: Parametric Bell 1 (`40..=16000 Hz`, `-18..=+18 dB`, Q `0.3..=12.0`)
/// - Band 3: Parametric Bell 2 (`100..=18000 Hz`, `-18..=+18 dB`, Q `0.3..=12.0`)
/// - Band 4: High Shelf (`2000..=18000 Hz`, `-18..=+18 dB`, Q `0.5..=2.0`)
/// - Band 5: Low-pass (`1000..=20000 Hz`, Q `0.5..=4.0`, enable bool)
#[derive(Clone, Debug)]
pub struct ParametricEq {
    configs: [BandConfig; 6],
    biquads: [Biquad; 6],
}

impl Default for ParametricEq {
    fn default() -> Self {
        Self::new()
    }
}

impl ParametricEq {
    pub fn new() -> Self {
        let configs = [
            BandConfig {
                enabled: false,
                freq_hz: 30.0,
                gain_db: 0.0,
                q: 0.707,
            },
            BandConfig {
                enabled: true,
                freq_hz: 100.0,
                gain_db: 0.0,
                q: 0.707,
            },
            BandConfig {
                enabled: true,
                freq_hz: 500.0,
                gain_db: 0.0,
                q: 1.0,
            },
            BandConfig {
                enabled: true,
                freq_hz: 2500.0,
                gain_db: 0.0,
                q: 1.0,
            },
            BandConfig {
                enabled: true,
                freq_hz: 8000.0,
                gain_db: 0.0,
                q: 0.707,
            },
            BandConfig {
                enabled: false,
                freq_hz: 18000.0,
                gain_db: 0.0,
                q: 0.707,
            },
        ];
        Self {
            configs,
            biquads: [Biquad::default(); 6],
        }
    }

    pub fn band_kind(band: usize) -> Option<BandKind> {
        BAND_KINDS.get(band).copied()
    }

    pub fn band(&self, band: usize) -> Option<BandConfig> {
        self.configs.get(band).copied()
    }

    pub fn set_band(&mut self, band: usize, config: BandConfig) -> Result<(), Error> {
        if band >= 6 {
            return Err(Error::Band);
        }
        validate_band(band, config)?;
        self.configs[band] = config;
        if config.enabled {
            self.biquads[band].configure(
                BAND_KINDS[band],
                config.freq_hz,
                config.gain_db,
                config.q,
            );
        } else {
            self.biquads[band].set_bypass();
        }
        Ok(())
    }

    pub fn set_high_pass(&mut self, enabled: bool, freq_hz: f32, q: f32) -> Result<(), Error> {
        self.set_band(
            0,
            BandConfig {
                enabled,
                freq_hz,
                gain_db: 0.0,
                q,
            },
        )
    }

    pub fn set_low_shelf(&mut self, freq_hz: f32, gain_db: f32, q: f32) -> Result<(), Error> {
        self.set_band(
            1,
            BandConfig {
                enabled: true,
                freq_hz,
                gain_db,
                q,
            },
        )
    }

    pub fn set_bell_1(&mut self, freq_hz: f32, gain_db: f32, q: f32) -> Result<(), Error> {
        self.set_band(
            2,
            BandConfig {
                enabled: true,
                freq_hz,
                gain_db,
                q,
            },
        )
    }

    pub fn set_bell_2(&mut self, freq_hz: f32, gain_db: f32, q: f32) -> Result<(), Error> {
        self.set_band(
            3,
            BandConfig {
                enabled: true,
                freq_hz,
                gain_db,
                q,
            },
        )
    }

    pub fn set_high_shelf(&mut self, freq_hz: f32, gain_db: f32, q: f32) -> Result<(), Error> {
        self.set_band(
            4,
            BandConfig {
                enabled: true,
                freq_hz,
                gain_db,
                q,
            },
        )
    }

    pub fn set_low_pass(&mut self, enabled: bool, freq_hz: f32, q: f32) -> Result<(), Error> {
        self.set_band(
            5,
            BandConfig {
                enabled,
                freq_hz,
                gain_db: 0.0,
                q,
            },
        )
    }

    /// Apply a detected resonance cut directly to Bell 1 (band 2) or Bell 2 (band 3).
    pub fn apply_resonance_cut(
        &mut self,
        band: usize,
        candidate: ResonanceCandidate,
    ) -> Result<(), Error> {
        if band != 2 && band != 3 {
            return Err(Error::Band);
        }
        let (f_min, f_max) = if band == 2 {
            (40.0, 16000.0)
        } else {
            (100.0, 18000.0)
        };
        self.set_band(
            band,
            BandConfig {
                enabled: true,
                freq_hz: candidate.freq_hz.clamp(f_min, f_max),
                gain_db: candidate.cut_db.clamp(-18.0, 18.0),
                q: candidate.q.clamp(0.3, 12.0),
            },
        )
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, mut lr: [f32; 2]) -> [f32; 2] {
        for b in &mut self.biquads {
            lr = b.process(lr);
        }
        lr
    }

    #[inline]
    pub fn process_frame(&mut self, frame: Frame) -> Frame {
        if self.biquads.iter().all(|b| b.bypass) {
            return frame;
        }
        f32_to_frame(self.process_stereo_f32(frame_to_f32(frame)))
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        if self.biquads.iter().all(|b| b.bypass) {
            return;
        }
        for f in frames {
            *f = f32_to_frame(self.process_stereo_f32(frame_to_f32(*f)));
        }
    }

    pub fn reset(&mut self) {
        for b in &mut self.biquads {
            b.reset();
        }
    }

    /// Exact cascaded complex magnitude response in dB at each requested frequency in Hz.
    pub fn response_curve_db(&self, freqs_hz: &[f32], out_db: &mut [f32]) {
        let nyquist_guard = RATE as f32 * 0.4999;
        for (&f_raw, out) in freqs_hz.iter().zip(out_db.iter_mut()) {
            let f = if f_raw.is_finite() {
                f_raw.clamp(1.0, nyquist_guard)
            } else {
                1000.0
            };
            let w = TAU * f / RATE as f32;
            let s = sin_f32(0.5 * w) as f64;
            let phi = s * s;
            let mut mag_sq = 1.0f64;
            for b in &self.biquads {
                mag_sq *= b.magnitude_sq(phi);
            }
            *out = 10.0 * log10_f32((mag_sq as f32).max(1e-24));
        }
    }
}

fn validate_band(band: usize, cfg: BandConfig) -> Result<(), Error> {
    if !cfg.freq_hz.is_finite() || !cfg.gain_db.is_finite() || !cfg.q.is_finite() {
        return Err(Error::Frequency);
    }
    let (f_min, f_max, q_min, q_max, has_gain) = match band {
        0 => (20.0, 1000.0, 0.5, 4.0, false),
        1 => (30.0, 500.0, 0.5, 2.0, true),
        2 => (40.0, 16000.0, 0.3, 12.0, true),
        3 => (100.0, 18000.0, 0.3, 12.0, true),
        4 => (2000.0, 18000.0, 0.5, 2.0, true),
        5 => (1000.0, 20000.0, 0.5, 4.0, false),
        _ => return Err(Error::Band),
    };
    if cfg.freq_hz < f_min || cfg.freq_hz > f_max {
        return Err(Error::Frequency);
    }
    if cfg.q < q_min || cfg.q > q_max {
        return Err(Error::Q);
    }
    if has_gain && (cfg.gain_db < -18.0 || cfg.gain_db > 18.0) {
        return Err(Error::Gain);
    }
    Ok(())
}

/// Hann-windowed radix-2 in-place stereo power (`L^2 + R^2`) FFT spectrum analyzer.
#[derive(Clone, Debug)]
pub struct SpectrumAnalyzer {
    fft_size: usize,
    window: Vec<f32>,
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    ring_pos: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    power: Vec<f32>,
    smoothing: f32,
}

impl SpectrumAnalyzer {
    pub const DEFAULT_FFT_SIZE: usize = 512;

    /// Create a 256 or 512-point stereo power FFT spectrum analyzer.
    pub fn new(fft_size: usize) -> Result<Self, Error> {
        if fft_size != 256 && fft_size != 512 {
            return Err(Error::Size);
        }
        let bins = fft_size / 2 + 1;

        let mut window = Vec::new();
        window
            .try_reserve_exact(fft_size)
            .map_err(|_| Error::Memory)?;
        for i in 0..fft_size {
            window.push(0.5 * (1.0 - cos_f32(TAU * i as f32 / fft_size as f32)));
        }

        let mut ring_l = Vec::new();
        ring_l
            .try_reserve_exact(fft_size)
            .map_err(|_| Error::Memory)?;
        ring_l.resize(fft_size, 0.0);

        let mut ring_r = Vec::new();
        ring_r
            .try_reserve_exact(fft_size)
            .map_err(|_| Error::Memory)?;
        ring_r.resize(fft_size, 0.0);

        let mut re = Vec::new();
        re.try_reserve_exact(fft_size).map_err(|_| Error::Memory)?;
        re.resize(fft_size, 0.0);

        let mut im = Vec::new();
        im.try_reserve_exact(fft_size).map_err(|_| Error::Memory)?;
        im.resize(fft_size, 0.0);

        let mut power = Vec::new();
        power.try_reserve_exact(bins).map_err(|_| Error::Memory)?;
        power.resize(bins, 0.0);

        Ok(Self {
            fft_size,
            window,
            ring_l,
            ring_r,
            ring_pos: 0,
            re,
            im,
            power,
            smoothing: 0.0,
        })
    }

    pub fn fft_size(&self) -> usize {
        self.fft_size
    }

    pub fn bin_count(&self) -> usize {
        self.fft_size / 2 + 1
    }

    /// Set exponential power smoothing in `0.0..=0.98` (0.0 = instantaneous window).
    pub fn set_smoothing(&mut self, smoothing: f32) -> Result<(), Error> {
        if !smoothing.is_finite() || !(0.0..=0.98).contains(&smoothing) {
            return Err(Error::Gain);
        }
        self.smoothing = smoothing;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.ring_l.fill(0.0);
        self.ring_r.fill(0.0);
        self.ring_pos = 0;
        self.power.fill(0.0);
    }

    #[inline]
    pub fn push_frame(&mut self, frame: Frame) {
        let [l, r] = frame_to_f32(frame);
        self.ring_l[self.ring_pos] = l;
        self.ring_r[self.ring_pos] = r;
        self.ring_pos = (self.ring_pos + 1) % self.fft_size;
    }

    pub fn push_frames(&mut self, frames: &[Frame]) {
        for &f in frames {
            self.push_frame(f);
        }
    }

    /// Compute stereo power spectrum in dBFS (`0.0 dBFS` = full-scale sine) from the ring buffer.
    /// Returns the number of bins written (`min(out_db.len(), bin_count())`).
    pub fn compute_spectrum_db(&mut self, out_db: &mut [f32]) -> usize {
        let n = self.fft_size;
        let mut idx = self.ring_pos;
        for i in 0..n {
            let w = self.window[i];
            self.re[i] = self.ring_l[idx] * w;
            self.im[i] = self.ring_r[idx] * w;
            idx = (idx + 1) % n;
        }
        self.run_stereo_power_fft(out_db)
    }

    /// Analyze a slice of frames directly (most recent `fft_size` frames, zero-padded if shorter)
    /// without altering the internal streaming ring buffer, returning bins written.
    pub fn analyze_frames(&mut self, frames: &[Frame], out_db: &mut [f32]) -> usize {
        let n = self.fft_size;
        let src = if frames.len() > n {
            &frames[frames.len() - n..]
        } else {
            frames
        };
        let pad = n - src.len();
        for i in 0..pad {
            self.re[i] = 0.0;
            self.im[i] = 0.0;
        }
        for (j, &f) in src.iter().enumerate() {
            let i = pad + j;
            let [l, r] = frame_to_f32(f);
            let w = self.window[i];
            self.re[i] = l * w;
            self.im[i] = r * w;
        }
        self.run_stereo_power_fft(out_db)
    }

    fn run_stereo_power_fft(&mut self, out_db: &mut [f32]) -> usize {
        let n = self.fft_size;
        radix2_fft_in_place(&mut self.re, &mut self.im);

        // Exploiting complex linearity Z[k] = L[k] + j*R[k]:
        // |L[k]|^2 + |R[k]|^2 = (|Z[k]|^2 + |Z[N-k]|^2) / 2.
        let half = n / 2;
        let sine_norm = (n as f32) * 0.25;
        let sine_norm_sq = sine_norm * sine_norm;
        let dc_norm = (n as f32) * 0.5;
        let dc_norm_sq = dc_norm * dc_norm;

        let sm = self.smoothing;
        let keep = 1.0 - sm;

        // Bin 0 (DC)
        let p0 = (self.re[0] * self.re[0] + self.im[0] * self.im[0]) * 0.5 / dc_norm_sq;
        self.power[0] = sm * self.power[0] + keep * p0;

        // Bins 1..half
        for k in 1..half {
            let rk = self.re[k];
            let ik = self.im[k];
            let rnk = self.re[n - k];
            let ink = self.im[n - k];
            let stereo_pow = 0.5 * ((rk * rk + ik * ik) + (rnk * rnk + ink * ink));
            let p = (stereo_pow * 0.5) / sine_norm_sq;
            self.power[k] = sm * self.power[k] + keep * p;
        }

        // Bin half (Nyquist)
        let ph = (self.re[half] * self.re[half] + self.im[half] * self.im[half]) * 0.5 / dc_norm_sq;
        self.power[half] = sm * self.power[half] + keep * ph;

        let count = out_db.len().min(half + 1);
        for k in 0..count {
            let p = self.power[k];
            out_db[k] = if p > 1e-12 {
                10.0 * log10_f32(p)
            } else {
                -120.0
            };
        }
        count
    }
}

fn radix2_fft_in_place(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert_eq!(n, im.len());
    debug_assert!(n.is_power_of_two());

    // Bit-reversal permutation
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Cooley-Tukey decimation-in-time butterflies
    let mut len = 2usize;
    while len <= n {
        let half = len / 2;
        let angle_step = -2.0 * PI / len as f32;
        for start in (0..n).step_by(len) {
            for k in 0..half {
                let angle = angle_step * k as f32;
                let wr = cos_f32(angle);
                let wi = sin_f32(angle);
                let idx_e = start + k;
                let idx_o = idx_e + half;
                let tr = wr * re[idx_o] - wi * im[idx_o];
                let ti = wr * im[idx_o] + wi * re[idx_o];
                re[idx_o] = re[idx_e] - tr;
                im[idx_o] = im[idx_e] - ti;
                re[idx_e] += tr;
                im[idx_e] += ti;
            }
        }
        len <<= 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResonanceCandidate {
    pub freq_hz: f32,
    /// Conservative negative bell cut in dB, capped at `-6.0 dB` (`-6.0 <= cut_db < 0.0`).
    pub cut_db: f32,
    /// Suggested bell Q factor in `4.0..=8.0`.
    pub q: f32,
    /// Height in dB above local spectral neighborhood.
    pub prominence_db: f32,
}

/// Inspect a spectrum in dBFS, suppress silence (`< -55 dBFS`) and broadband tilt,
/// and propose up to 3 conservative narrow-peak bell cuts (`cut_db` capped at `-6.0 dB`, `q` in `4.0..=8.0`).
pub fn detect_resonances(spectrum_db: &[f32], sample_rate: u32) -> [Option<ResonanceCandidate>; 3] {
    let mut result = [None; 3];
    if spectrum_db.len() < 16 || sample_rate == 0 {
        return result;
    }

    let num_bins = spectrum_db.len();
    let fft_size = (num_bins - 1) * 2;
    let bin_hz = sample_rate as f32 / fft_size as f32;

    const SILENCE_FLOOR_DB: f32 = -55.0;
    const MIN_PROMINENCE_DB: f32 = 6.0;
    const MIN_SIDE_DROP_DB: f32 = 4.5;

    let min_bin = ((60.0 / bin_hz) as usize).max(2);
    let max_bin = (((18000.0 / bin_hz) as usize).min(num_bins.saturating_sub(3))).max(min_bin);

    // Track already selected bins so a single wide lobe cannot occupy multiple slots.
    let mut chosen_bins = [0usize; 3];
    let mut chosen_count = 0usize;

    for slot in 0..3 {
        let mut best: Option<(usize, ResonanceCandidate)> = None;

        for k in min_bin..=max_bin {
            if chosen_bins[..chosen_count]
                .iter()
                .any(|&c| c.abs_diff(k) <= 3)
            {
                continue;
            }

            let center = spectrum_db[k];
            if !center.is_finite() || center < SILENCE_FLOOR_DB {
                continue;
            }
            let prev = spectrum_db[k - 1];
            let next = spectrum_db[k + 1];
            if !prev.is_finite() || !next.is_finite() {
                continue;
            }
            if !(center >= prev && center >= next && (center > prev || center > next)) {
                continue;
            }

            // Estimate local flanking baseline outside the Hann main lobe (k +- 2..6).
            let left_start = k.saturating_sub(6).max(1);
            let left_end = k.saturating_sub(2);
            let right_start = (k + 2).min(num_bins - 1);
            let right_end = (k + 6).min(num_bins - 1);

            let mut left_sum = 0.0f32;
            let mut left_n = 0usize;
            for &v in &spectrum_db[left_start..=left_end] {
                if v.is_finite() {
                    left_sum += v;
                    left_n += 1;
                }
            }
            let mut right_sum = 0.0f32;
            let mut right_n = 0usize;
            for &v in &spectrum_db[right_start..=right_end] {
                if v.is_finite() {
                    right_sum += v;
                    right_n += 1;
                }
            }
            if left_n == 0 || right_n == 0 {
                continue;
            }

            let left_avg = (left_sum / left_n as f32).max(SILENCE_FLOOR_DB);
            let right_avg = (right_sum / right_n as f32).max(SILENCE_FLOOR_DB);
            let baseline = 0.5 * (left_avg + right_avg);

            let drop_left = center - left_avg;
            let drop_right = center - right_avg;
            let prominence = center - baseline;

            if prominence < MIN_PROMINENCE_DB
                || drop_left < MIN_SIDE_DROP_DB
                || drop_right < MIN_SIDE_DROP_DB
            {
                continue;
            }

            // Sub-bin parabolic frequency interpolation around k.
            let denom = prev - 2.0 * center + next;
            let delta = if abs_f32(denom) > 1e-6 {
                ((prev - next) / (2.0 * denom)).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            let freq_hz = ((k as f32 + delta) * bin_hz).clamp(40.0, 18000.0);

            // Narrower peaks (sharper immediate skirt drop) receive higher Q within 4.0..=8.0.
            let immediate_drop = center - 0.5 * (prev + next);
            let q = (4.0 + (immediate_drop / prominence).clamp(0.0, 1.0) * 4.0).clamp(4.0, 8.0);
            let cut_db = -(prominence * 0.5).clamp(1.5, 6.0);

            let candidate = ResonanceCandidate {
                freq_hz,
                cut_db,
                q,
                prominence_db: prominence,
            };

            let replace = match best {
                None => true,
                Some((_, cur)) => candidate.prominence_db > cur.prominence_db,
            };
            if replace {
                best = Some((k, candidate));
            }
        }

        if let Some((bin, cand)) = best {
            chosen_bins[chosen_count] = bin;
            chosen_count += 1;
            result[slot] = Some(cand);
        } else {
            break;
        }
    }

    result
}
