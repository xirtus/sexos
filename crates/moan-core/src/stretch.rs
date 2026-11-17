// SPDX-License-Identifier: GPL-3.0-only
//! Signalsmith-inspired pitch-preserving elastic time-stretcher & pitch-shifter.
//! Supports `0.25x..=4.0x` tempo scaling and `-12.0..=+12.0` semitone pitch shifting
//! on stereo 48 kHz audio using phase-locked stereo WSOLA with normalized cross-correlation,
//! cubic Catmull-Rom resampling, anti-alias filtering, and optional vocal formant preservation.
//! Pure `#![no_std]` + `alloc`, zero `unsafe`.

use crate::{
    dsp_math::{abs_f32, cos_f32, exp2_f32, floor_f32, round_f32, sin_f32, sqrt_f32, TAU},
    Frame, RATE,
};
use alloc::{vec, vec::Vec};

pub const MIN_TEMPO_RATIO: f32 = 0.25;
pub const MAX_TEMPO_RATIO: f32 = 4.0;
pub const MIN_PITCH_SEMITONES: f32 = -12.0;
pub const MAX_PITCH_SEMITONES: f32 = 12.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StretchError {
    InvalidParameter,
    Memory,
}

/// Configuration for [`ElasticStretch`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StretchConfig {
    /// Tempo multiplier in `0.25..=4.0`.
    /// `0.5` plays at half speed (2x duration); `2.0` plays at double speed (0.5x duration).
    pub tempo_ratio: f32,
    /// Pitch shift in `-12.0..=+12.0` semitones (`0.0` = pitch-preserving time stretch).
    pub pitch_semitones: f32,
    /// Preserves vocal tract spectral envelope when shifting pitch to avoid chipmunk timbre.
    pub formant_preserve: bool,
    /// Audio sample rate in Hz (default 48,000 Hz).
    pub sample_rate: u32,
}

impl Default for StretchConfig {
    fn default() -> Self {
        Self {
            tempo_ratio: 1.0,
            pitch_semitones: 0.0,
            formant_preserve: false,
            sample_rate: RATE,
        }
    }
}

impl StretchConfig {
    pub fn validate(&self) -> Result<(), StretchError> {
        if !self.tempo_ratio.is_finite()
            || !(MIN_TEMPO_RATIO..=MAX_TEMPO_RATIO).contains(&self.tempo_ratio)
        {
            return Err(StretchError::InvalidParameter);
        }
        if !self.pitch_semitones.is_finite()
            || !(MIN_PITCH_SEMITONES..=MAX_PITCH_SEMITONES).contains(&self.pitch_semitones)
        {
            return Err(StretchError::InvalidParameter);
        }
        if !(8000..=192_000).contains(&self.sample_rate) {
            return Err(StretchError::InvalidParameter);
        }
        Ok(())
    }
}

/// Phase-locked stereo WSOLA time-stretcher & pitch-shifter.
#[derive(Clone, Debug)]
pub struct ElasticStretch {
    config: StretchConfig,
}

impl ElasticStretch {
    pub fn new(tempo_ratio: f32, pitch_semitones: f32) -> Result<Self, StretchError> {
        Self::with_config(StretchConfig {
            tempo_ratio,
            pitch_semitones,
            formant_preserve: false,
            sample_rate: RATE,
        })
    }

    /// Creates a formant-preserving vocal pitch/time processor (prevents chipmunk coloration).
    pub fn vocal(tempo_ratio: f32, pitch_semitones: f32) -> Result<Self, StretchError> {
        Self::with_config(StretchConfig {
            tempo_ratio,
            pitch_semitones,
            formant_preserve: true,
            sample_rate: RATE,
        })
    }

    pub fn with_config(config: StretchConfig) -> Result<Self, StretchError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn config(&self) -> StretchConfig {
        self.config
    }

    /// Processes stereo 48 kHz `Frame`s and returns the time-stretched / pitch-shifted result.
    pub fn process_frames(&self, source: &[Frame]) -> Result<Vec<Frame>, StretchError> {
        if source.is_empty() {
            return Ok(Vec::new());
        }
        if abs_f32(self.config.tempo_ratio - 1.0) < 1e-6
            && abs_f32(self.config.pitch_semitones) < 1e-4
        {
            let mut out = Vec::new();
            out.try_reserve_exact(source.len())
                .map_err(|_| StretchError::Memory)?;
            out.extend_from_slice(source);
            return Ok(out);
        }

        let mut f32_in = Vec::new();
        f32_in
            .try_reserve_exact(source.len())
            .map_err(|_| StretchError::Memory)?;
        for &f in source {
            f32_in.push([f[0] as f32 / 32768.0, f[1] as f32 / 32768.0]);
        }

        let processed = self.process_f32(&f32_in)?;
        let mut out = Vec::new();
        out.try_reserve_exact(processed.len())
            .map_err(|_| StretchError::Memory)?;
        for s in processed {
            out.push([
                round_f32(s[0] * 32768.0).clamp(-32768.0, 32767.0) as i16,
                round_f32(s[1] * 32768.0).clamp(-32768.0, 32767.0) as i16,
            ]);
        }
        Ok(out)
    }

    /// Processes stereo `f32` audio buffers without modifying pitch when `pitch_semitones == 0.0`.
    pub fn process_f32(&self, source: &[[f32; 2]]) -> Result<Vec<[f32; 2]>, StretchError> {
        if source.is_empty() {
            return Ok(Vec::new());
        }
        if abs_f32(self.config.tempo_ratio - 1.0) < 1e-6
            && abs_f32(self.config.pitch_semitones) < 1e-4
        {
            let mut out = Vec::new();
            out.try_reserve_exact(source.len())
                .map_err(|_| StretchError::Memory)?;
            out.extend_from_slice(source);
            return Ok(out);
        }

        let target_len = (round_f32(source.len() as f32 / self.config.tempo_ratio) as usize).max(1);
        let pitch_ratio = exp2_f32(self.config.pitch_semitones / 12.0);
        let pitch_active = abs_f32(pitch_ratio - 1.0) > 1e-4;

        if !pitch_active {
            return wsola_phase_locked(source, target_len);
        }

        // To shift pitch by `pitch_ratio` and finish at `target_len`, first time-stretch to
        // `target_len * pitch_ratio`, then resample at step `pitch_ratio`.
        let pre_len = (round_f32(target_len as f32 * pitch_ratio) as usize).max(1);
        let mut stretched = wsola_phase_locked(source, pre_len)?;

        // Anti-alias low-pass before upward pitch shifting to prevent Nyquist foldback.
        if pitch_ratio > 1.03 {
            let cutoff = (0.44 * self.config.sample_rate as f32 / pitch_ratio)
                .clamp(2000.0, self.config.sample_rate as f32 * 0.45);
            lowpass_2pole_inplace(&mut stretched, cutoff, self.config.sample_rate as f32);
        }

        let mut resampled = resample_cubic(&stretched, pitch_ratio, target_len)?;

        if self.config.formant_preserve {
            match_spectral_envelope(source, &mut resampled, self.config.sample_rate as f32);
        }

        Ok(resampled)
    }
}

/// Phase-locked stereo WSOLA using 75%-overlap Hann grains and coarse-to-fine Normalized
/// Cross-Correlation (NCC) against the natural source continuation (`prev_in + hop`).
fn wsola_phase_locked(source: &[[f32; 2]], target_len: usize) -> Result<Vec<[f32; 2]>, StretchError> {
    if target_len == 0 || source.is_empty() {
        return Ok(Vec::new());
    }
    if source.len() == target_len {
        let mut out = Vec::new();
        out.try_reserve_exact(source.len())
            .map_err(|_| StretchError::Memory)?;
        out.extend_from_slice(source);
        return Ok(out);
    }

    let win_size = if source.len() >= 4096 {
        2048usize
    } else if source.len() >= 2048 {
        1024usize
    } else if source.len() >= 1024 {
        512usize
    } else {
        return resample_cubic_to_len(source, target_len);
    };

    let hop_out = win_size / 4; // 75% overlap: periodic Hann sums to constant 2.0
    let search_range = win_size / 2;

    let mut window = Vec::new();
    window
        .try_reserve_exact(win_size)
        .map_err(|_| StretchError::Memory)?;
    for i in 0..win_size {
        window.push(0.5 * (1.0 - cos_f32(TAU * i as f32 / win_size as f32)));
    }

    // Mono mix for phase-locked stereo-coherent correlation
    let mut mono = Vec::new();
    mono.try_reserve_exact(source.len())
        .map_err(|_| StretchError::Memory)?;
    for &s in source {
        let l = if s[0].is_finite() { s[0] } else { 0.0 };
        let r = if s[1].is_finite() { s[1] } else { 0.0 };
        mono.push(0.5 * (l + r));
    }

    let buf_len = target_len + win_size;
    let mut acc_l = vec![0.0f32; buf_len];
    let mut acc_r = vec![0.0f32; buf_len];
    let mut weight = vec![0.0f32; buf_len];

    let max_in = source.len().saturating_sub(win_size);
    let stretch_ratio = target_len as f32 / source.len() as f32;

    let mut out_pos = 0usize;
    let mut prev_in = 0usize;
    let corr_len = (win_size / 2).min(512);

    while out_pos < target_len {
        let in_nominal = (floor_f32(out_pos as f32 / stretch_ratio) as usize).min(max_in);

        let actual_in = if out_pos == 0 {
            0
        } else {
            // Natural continuation of the previous grain in the clean original waveform.
            let natural_ref = (prev_in + hop_out).min(max_in);
            let lo = in_nominal.saturating_sub(search_range);
            let hi = (in_nominal + search_range).min(max_in);

            if lo >= hi || natural_ref + corr_len > mono.len() {
                in_nominal
            } else {
                // Coarse-to-fine NCC search
                let mut best_pos = in_nominal.clamp(lo, hi);
                let mut best_ncc = -2.0f32;

                let mut pos = lo;
                while pos <= hi {
                    let ncc = normalized_cross_corr(&mono, natural_ref, pos, corr_len, 4);
                    if ncc > best_ncc {
                        best_ncc = ncc;
                        best_pos = pos;
                    }
                    pos += 8;
                }

                let fine_lo = best_pos.saturating_sub(8).max(lo);
                let fine_hi = (best_pos + 8).min(hi);
                for pos in fine_lo..=fine_hi {
                    let ncc = normalized_cross_corr(&mono, natural_ref, pos, corr_len, 2);
                    if ncc > best_ncc {
                        best_ncc = ncc;
                        best_pos = pos;
                    }
                }
                best_pos
            }
        };

        let copy_len = win_size.min(source.len() - actual_in);
        for k in 0..copy_len {
            let w = window[k];
            let s = source[actual_in + k];
            acc_l[out_pos + k] += (if s[0].is_finite() { s[0] } else { 0.0 }) * w;
            acc_r[out_pos + k] += (if s[1].is_finite() { s[1] } else { 0.0 }) * w;
            weight[out_pos + k] += w;
        }

        prev_in = actual_in;
        out_pos += hop_out;
    }

    let mut out = Vec::new();
    out.try_reserve_exact(target_len)
        .map_err(|_| StretchError::Memory)?;
    for i in 0..target_len {
        let w = weight[i];
        let inv = if w > 1e-5 { 1.0 / w } else { 1.0 };
        out.push([
            (acc_l[i] * inv).clamp(-1.0, 1.0),
            (acc_r[i] * inv).clamp(-1.0, 1.0),
        ]);
    }
    Ok(out)
}

#[inline]
fn normalized_cross_corr(
    mono: &[f32],
    ref_pos: usize,
    cand_pos: usize,
    len: usize,
    step: usize,
) -> f32 {
    let max_len = len
        .min(mono.len().saturating_sub(ref_pos))
        .min(mono.len().saturating_sub(cand_pos));
    if max_len == 0 {
        return -1.0;
    }
    let mut dot = 0.0f32;
    let mut e_ref = 0.0f32;
    let mut e_cand = 0.0f32;
    let mut i = 0usize;
    while i < max_len {
        let a = mono[ref_pos + i];
        let b = mono[cand_pos + i];
        dot += a * b;
        e_ref += a * a;
        e_cand += b * b;
        i += step;
    }
    let denom = sqrt_f32(e_ref * e_cand);
    if denom > 1e-8 {
        dot / denom
    } else if e_ref < 1e-8 && e_cand < 1e-8 {
        1.0
    } else {
        0.0
    }
}

/// 4-point Catmull-Rom cubic interpolation at fractional index `pos`.
#[inline]
fn sample_cubic(src: &[[f32; 2]], pos: f32) -> [f32; 2] {
    let last = src.len() - 1;
    let i1 = (floor_f32(pos) as isize).clamp(0, last as isize) as usize;
    let t = (pos - i1 as f32).clamp(0.0, 1.0);
    let i0 = i1.saturating_sub(1);
    let i2 = (i1 + 1).min(last);
    let i3 = (i1 + 2).min(last);

    let t2 = t * t;
    let t3 = t2 * t;
    let mut out = [0.0f32; 2];
    for c in 0..2 {
        let p0 = src[i0][c];
        let p1 = src[i1][c];
        let p2 = src[i2][c];
        let p3 = src[i3][c];
        let v = 0.5
            * ((2.0 * p1)
                + (-p0 + p2) * t
                + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
                + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3);
        out[c] = v.clamp(-1.0, 1.0);
    }
    out
}

fn resample_cubic(
    src: &[[f32; 2]],
    step: f32,
    target_len: usize,
) -> Result<Vec<[f32; 2]>, StretchError> {
    if src.is_empty() || target_len == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    out.try_reserve_exact(target_len)
        .map_err(|_| StretchError::Memory)?;
    for i in 0..target_len {
        let pos = i as f32 * step;
        out.push(sample_cubic(src, pos));
    }
    Ok(out)
}

fn resample_cubic_to_len(
    src: &[[f32; 2]],
    target_len: usize,
) -> Result<Vec<[f32; 2]>, StretchError> {
    if src.is_empty() || target_len == 0 {
        return Ok(Vec::new());
    }
    if src.len() == 1 {
        let mut out = Vec::new();
        out.try_reserve_exact(target_len)
            .map_err(|_| StretchError::Memory)?;
        out.resize(target_len, src[0]);
        return Ok(out);
    }
    let step = (src.len() - 1) as f32 / (target_len.max(1) as f32);
    resample_cubic(src, step, target_len)
}

fn lowpass_2pole_inplace(buf: &mut [[f32; 2]], cutoff_hz: f32, sample_rate: f32) {
    let w0 = TAU * cutoff_hz / sample_rate;
    let cos_w0 = cos_f32(w0);
    let alpha = sin_f32(w0) / (2.0 * 0.70710678);
    let a0 = 1.0 + alpha;
    let b1 = (1.0 - cos_w0) / a0;
    let b0 = 0.5 * b1;
    let b2 = b0;
    let a1 = (-2.0 * cos_w0) / a0;
    let a2 = (1.0 - alpha) / a0;

    let mut z1 = [0.0f32; 2];
    let mut z2 = [0.0f32; 2];
    for f in buf.iter_mut() {
        for c in 0..2 {
            let x = f[c];
            let y = b0 * x + z1[c];
            z1[c] = b1 * x - a1 * y + z2[c];
            z2[c] = b2 * x - a2 * y;
            f[c] = y;
        }
    }
}

/// 4-band allpass-aligned Linkwitz-Riley spectral envelope matcher to preserve vocal formants
/// when shifting pitch.
fn match_spectral_envelope(orig: &[[f32; 2]], shifted: &mut [[f32; 2]], sample_rate: f32) {
    if orig.is_empty() || shifted.is_empty() {
        return;
    }
    let env_orig = band_rms_4(orig, sample_rate);
    let env_shift = band_rms_4(shifted, sample_rate);

    let mut gains = [1.0f32; 4];
    for b in 0..4 {
        if env_shift[b] > 1e-5 && env_orig[b] > 1e-5 {
            gains[b] = (env_orig[b] / env_shift[b]).clamp(0.35, 2.8);
        }
    }

    let mut bank = BandSplit4::new(sample_rate);
    for f in shifted.iter_mut() {
        let bands = bank.split(*f);
        f[0] = (bands[0][0] * gains[0]
            + bands[1][0] * gains[1]
            + bands[2][0] * gains[2]
            + bands[3][0] * gains[3])
            .clamp(-1.0, 1.0);
        f[1] = (bands[0][1] * gains[0]
            + bands[1][1] * gains[1]
            + bands[2][1] * gains[2]
            + bands[3][1] * gains[3])
            .clamp(-1.0, 1.0);
    }
}

fn band_rms_4(buf: &[[f32; 2]], sample_rate: f32) -> [f32; 4] {
    let mut bank = BandSplit4::new(sample_rate);
    let mut sum_sq = [0.0f64; 4];
    for &f in buf {
        let bands = bank.split(f);
        for (b, s) in bands.iter().enumerate() {
            sum_sq[b] += 0.5 * ((s[0] as f64) * (s[0] as f64) + (s[1] as f64) * (s[1] as f64));
        }
    }
    let n = (buf.len() as f64).max(1.0);
    core::array::from_fn(|b| sqrt_f32((sum_sq[b] / n) as f32))
}

struct Lr2 {
    lp_b: [f32; 3],
    hp_b: [f32; 3],
    a: [f32; 2],
    lz1: [f32; 2],
    lz2: [f32; 2],
    hz1: [f32; 2],
    hz2: [f32; 2],
}

impl Lr2 {
    fn new(freq_hz: f32, sample_rate: f32) -> Self {
        let fc = freq_hz.clamp(20.0, sample_rate * 0.45);
        let w0 = TAU * fc / sample_rate;
        let cos_w0 = cos_f32(w0);
        let alpha = sin_f32(w0); // Q = 0.5 -> 2*Q = 1.0
        let a0 = 1.0 + alpha;
        let lb1 = (1.0 - cos_w0) / a0;
        let hb0 = (0.5 * (1.0 + cos_w0)) / a0;
        Self {
            lp_b: [0.5 * lb1, lb1, 0.5 * lb1],
            hp_b: [hb0, -(1.0 + cos_w0) / a0, hb0],
            a: [(-2.0 * cos_w0) / a0, (1.0 - alpha) / a0],
            lz1: [0.0; 2],
            lz2: [0.0; 2],
            hz1: [0.0; 2],
            hz2: [0.0; 2],
        }
    }

    #[inline]
    fn split(&mut self, x: [f32; 2]) -> ([f32; 2], [f32; 2]) {
        let mut lo = [0.0f32; 2];
        let mut hi = [0.0f32; 2];
        for c in 0..2 {
            let v = x[c];
            let yl = self.lp_b[0] * v + self.lz1[c];
            self.lz1[c] = self.lp_b[1] * v - self.a[0] * yl + self.lz2[c];
            self.lz2[c] = self.lp_b[2] * v - self.a[1] * yl;

            let yh = self.hp_b[0] * v + self.hz1[c];
            self.hz1[c] = self.hp_b[1] * v - self.a[0] * yh + self.hz2[c];
            self.hz2[c] = self.hp_b[2] * v - self.a[1] * yh;

            lo[c] = yl;
            hi[c] = -yh;
        }
        (lo, hi)
    }
}

struct BandSplit4 {
    s1200: Lr2,
    s450: Lr2,
    s3500: Lr2,
}

impl BandSplit4 {
    fn new(sample_rate: f32) -> Self {
        Self {
            s1200: Lr2::new(1200.0, sample_rate),
            s450: Lr2::new(450.0, sample_rate),
            s3500: Lr2::new(3500.0, sample_rate),
        }
    }

    #[inline]
    fn split(&mut self, x: [f32; 2]) -> [[f32; 2]; 4] {
        let (l, h) = self.s1200.split(x);
        let (b0, b1) = self.s450.split(l);
        let (b2, b3) = self.s3500.split(h);
        [b0, b1, b2, b3]
    }
}
