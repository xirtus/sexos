//! Voice memo & podcast post-processing ("Quick Voice Fixes") and ITU-R BS.1770-4 /
//! EBU R128 loudness & 4x oversampled True-Peak readout for `AudioClip`.
//!
//! Pure `#![no_std]` + `alloc`, `#![forbid(unsafe_code)]`, powered by `libm`.

use alloc::vec::Vec;
use core::f64::consts::PI;

use crate::edit::AudioClip;
use crate::meter::MIN_DBFS;
use crate::SerenadeError;

/// Standard podcast integrated loudness target (-16.0 LUFS).
pub const PODCAST_TARGET_LUFS: f32 = -16.0;
/// Standard broadcast/podcast True-Peak brickwall ceiling (-1.0 dBTP).
pub const TRUE_PEAK_CEILING_DBTP: f32 = -1.0;

const K_OFFSET: f64 = -0.691;
const ABS_GATE_LUFS: f64 = -70.0;
const REL_GATE_OFFSET_LU: f64 = 10.0;

/// ITU-R BS.1770-4 / EBU R128 loudness and 4x oversampled True-Peak readout on an [`AudioClip`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoudnessReadout {
    /// Gated integrated loudness (`-70 LKFS` abs + `-10 dB` relative gate) in LUFS.
    pub integrated_lufs: f32,
    /// Maximum 400 ms momentary loudness in LUFS.
    pub momentary_max_lufs: f32,
    /// Maximum 3 s short-term loudness in LUFS.
    pub short_term_max_lufs: f32,
    /// 4x oversampled True-Peak level in dBTP.
    pub true_peak_dbtp: f32,
    /// 4x oversampled True-Peak linear amplitude.
    pub true_peak_linear: f32,
}

impl Default for LoudnessReadout {
    fn default() -> Self {
        Self {
            integrated_lufs: MIN_DBFS,
            momentary_max_lufs: MIN_DBFS,
            short_term_max_lufs: MIN_DBFS,
            true_peak_dbtp: MIN_DBFS,
            true_peak_linear: 0.0,
        }
    }
}

#[inline]
fn sanitize(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

#[inline]
fn db_to_lin(db: f32) -> f32 {
    if !db.is_finite() || db <= MIN_DBFS {
        0.0
    } else {
        libm::powf(10.0, db / 20.0)
    }
}

#[inline]
fn lin_to_db(lin: f32) -> f32 {
    let a = libm::fabsf(lin);
    if !a.is_finite() || a <= 1e-6 {
        MIN_DBFS
    } else {
        20.0 * libm::log10f(a)
    }
}

#[inline]
fn time_coeff(ms: f32, sample_rate: f32) -> f32 {
    let denom = ms.max(0.05) * 0.001 * sample_rate.max(1.0);
    libm::expf(-1.0 / denom)
}

#[inline]
fn mean_sq_to_lufs(mean_sq: f64) -> f32 {
    if !mean_sq.is_finite() || mean_sq <= 1e-12 {
        return MIN_DBFS;
    }
    let lufs = (K_OFFSET + 10.0 * libm::log10(mean_sq)) as f32;
    if lufs < MIN_DBFS {
        MIN_DBFS
    } else {
        lufs
    }
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

/// Validates clip channel count and sample rate before DSP processing.
fn validate_clip(clip: &AudioClip) -> Result<(usize, u32, usize), SerenadeError> {
    if clip.channels == 0 || clip.channels > 2 {
        return Err(SerenadeError::InvalidFormat(
            "channels must be 1 (mono) or 2 (stereo)",
        ));
    }
    if clip.sample_rate == 0 {
        return Err(SerenadeError::InvalidSampleRate);
    }
    let ch = clip.channels as usize;
    if clip.samples.len() % ch != 0 {
        return Err(SerenadeError::InvalidFormat(
            "samples length not aligned to channel count",
        ));
    }
    let frames = clip.samples.len() / ch;
    Ok((ch, clip.sample_rate, frames))
}

/// Multi-channel (1 or 2 ch) `f64` Direct Form II Transposed biquad.
#[derive(Clone, Debug)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: [f64; 2],
    z2: [f64; 2],
}

impl Biquad {
    fn notch(freq_hz: f64, q: f64, sr: f64) -> Self {
        let fc = freq_hz.clamp(5.0, sr * 0.45);
        let q = q.clamp(0.5, 100.0);
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = libm::cos(w0);
        let alpha = libm::sin(w0) / (2.0 * q);
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

    fn lowpass(freq_hz: f64, q: f64, sr: f64) -> Self {
        let fc = freq_hz.clamp(5.0, sr * 0.45);
        let q = q.clamp(0.25, 20.0);
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = libm::cos(w0);
        let alpha = libm::sin(w0) / (2.0 * q);
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

    fn highpass(freq_hz: f64, q: f64, sr: f64) -> Self {
        let fc = freq_hz.clamp(5.0, sr * 0.45);
        let q = q.clamp(0.25, 20.0);
        let w0 = 2.0 * PI * fc / sr;
        let cos_w0 = libm::cos(w0);
        let alpha = libm::sin(w0) / (2.0 * q);
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

    fn k_stage1(sample_rate: u32) -> Self {
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
        let k = libm::tan(PI * f0 / fs);
        let vh = libm::pow(10.0, g_db / 20.0);
        let vb = libm::pow(vh, 0.4996667741545416);
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

    fn k_stage2(sample_rate: u32) -> Self {
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
        let k = libm::tan(PI * f0 / fs);
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
    fn process_ch(&mut self, ch: usize, x: f64) -> f64 {
        let v = if x.is_finite() { x } else { 0.0 };
        let y = self.b0 * v + self.z1[ch];
        self.z1[ch] = self.b1 * v - self.a1 * y + self.z2[ch];
        self.z2[ch] = self.b2 * v - self.a2 * y;
        y
    }

    #[inline]
    fn process_frame(&mut self, frame: [f32; 2], ch_count: usize) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for c in 0..ch_count {
            out[c] = self.process_ch(c, sanitize(frame[c]) as f64) as f32;
        }
        out
    }

    fn reset(&mut self) {
        self.z1 = [0.0; 2];
        self.z2 = [0.0; 2];
    }
}

#[derive(Clone, Debug)]
struct Allpass1 {
    coeff: f64,
    x1: [f64; 2],
    y1: [f64; 2],
}

impl Allpass1 {
    fn new(freq_hz: f64, sr: f64) -> Self {
        let fc = freq_hz.clamp(5.0, sr * 0.45);
        let k = libm::tan(PI * fc / sr);
        Self {
            coeff: (k - 1.0) / (k + 1.0),
            x1: [0.0; 2],
            y1: [0.0; 2],
        }
    }

    #[inline]
    fn process_frame(&mut self, frame: [f32; 2], ch_count: usize) -> [f32; 2] {
        let mut out = [0.0f32; 2];
        for c in 0..ch_count {
            let x = sanitize(frame[c]) as f64;
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

#[derive(Clone, Debug)]
struct Lr2 {
    lp: Biquad,
    hp: Biquad,
}

impl Lr2 {
    fn new(freq_hz: f64, sr: f64) -> Self {
        Self {
            lp: Biquad::lowpass(freq_hz, 0.5, sr),
            hp: Biquad::highpass(freq_hz, 0.5, sr),
        }
    }

    #[inline]
    fn split(&mut self, frame: [f32; 2], ch_count: usize) -> ([f32; 2], [f32; 2]) {
        let low = self.lp.process_frame(frame, ch_count);
        let high = self.hp.process_frame(frame, ch_count);
        (low, [-high[0], -high[1]])
    }

    fn reset(&mut self) {
        self.lp.reset();
        self.hp.reset();
    }
}

struct TruePeak4x {
    phases: [[f32; 12]; 3],
    history: [[f32; 12]; 2],
    max_peak: f32,
}

impl TruePeak4x {
    fn new() -> Self {
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
                    bessel_i0(beta * libm::sqrt(1.0 - r * r)) / i0_beta
                } else {
                    0.0
                };
                let sinc = if m.abs() < 1e-12 {
                    1.0
                } else {
                    libm::sin(PI * m) / (PI * m)
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

    #[inline]
    fn process_frame(&mut self, frame: [f32; 2], ch_count: usize) -> f32 {
        let mut step_peak = 0.0f32;
        for c in 0..ch_count {
            let s = sanitize(frame[c]);
            for i in 0..11 {
                self.history[c][i] = self.history[c][i + 1];
            }
            self.history[c][11] = s;
            let p0 = libm::fabsf(self.history[c][5]);
            if p0 > step_peak {
                step_peak = p0;
            }
            for phase in &self.phases {
                let mut acc = 0.0f32;
                for k in 0..12 {
                    acc += self.history[c][k] * phase[k];
                }
                let pa = libm::fabsf(acc);
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

    fn flush(&mut self, ch_count: usize) -> f32 {
        for _ in 0..6 {
            self.process_frame([0.0, 0.0], ch_count);
        }
        self.max_peak
    }
}

/// Measures ITU-R BS.1770-4 / EBU R128 loudness and 4x True-Peak on an [`AudioClip`].
pub fn measure_loudness(clip: &AudioClip) -> Result<LoudnessReadout, SerenadeError> {
    let (ch, sr, total_frames) = validate_clip(clip)?;
    if total_frames == 0 {
        return Ok(LoudnessReadout::default());
    }

    let mut stage1 = Biquad::k_stage1(sr);
    let mut stage2 = Biquad::k_stage2(sr);
    let mut tp = TruePeak4x::new();

    let sub_block_frames = (sr as usize / 10).max(1);
    let mut sub_blocks = Vec::new();
    let expected_blocks = total_frames / sub_block_frames + 1;
    sub_blocks
        .try_reserve(expected_blocks)
        .map_err(|_| SerenadeError::AllocationFailed)?;

    let mut cur_sum_sq = 0.0f64;
    let mut cur_frames = 0usize;
    let mut total_sum_sq = 0.0f64;
    let mut momentary_max = MIN_DBFS;
    let mut short_term_max = MIN_DBFS;

    for frame_idx in 0..total_frames {
        let base = frame_idx * ch;
        let frame = [
            sanitize(clip.samples[base]),
            if ch > 1 { sanitize(clip.samples[base + 1]) } else { 0.0 },
        ];
        tp.process_frame(frame, ch);

        let mut frame_sq = 0.0f64;
        for c in 0..ch {
            let s1 = stage1.process_ch(c, frame[c] as f64);
            let s2 = stage2.process_ch(c, s1);
            frame_sq += s2 * s2;
        }
        cur_sum_sq += frame_sq;
        cur_frames += 1;
        total_sum_sq += frame_sq;

        if cur_frames == sub_block_frames {
            sub_blocks.push(cur_sum_sq / (sub_block_frames as f64));
            cur_sum_sq = 0.0;
            cur_frames = 0;

            let n = sub_blocks.len();
            if n >= 4 {
                let m = mean_sq_to_lufs(0.25 * sub_blocks[n - 4..n].iter().sum::<f64>());
                if m > momentary_max {
                    momentary_max = m;
                }
            }
            let st_len = n.min(30);
            let st = mean_sq_to_lufs(
                sub_blocks[n - st_len..n].iter().sum::<f64>() / (st_len as f64),
            );
            if st > short_term_max {
                short_term_max = st;
            }
        }
    }

    let tp_lin = tp.flush(ch);
    let tp_dbtp = lin_to_db(tp_lin);

    let integrated_lufs = if sub_blocks.len() < 4 {
        let lufs = mean_sq_to_lufs(total_sum_sq / (total_frames as f64));
        if momentary_max == MIN_DBFS {
            momentary_max = lufs;
        }
        if short_term_max == MIN_DBFS {
            short_term_max = lufs;
        }
        if lufs <= ABS_GATE_LUFS as f32 {
            MIN_DBFS
        } else {
            lufs
        }
    } else {
        let abs_thresh = libm::pow(10.0, (ABS_GATE_LUFS - K_OFFSET) / 10.0);
        let mut abs_sum = 0.0f64;
        let mut abs_count = 0usize;
        for w in sub_blocks.windows(4) {
            let z = 0.25 * (w[0] + w[1] + w[2] + w[3]);
            if z > abs_thresh {
                abs_sum += z;
                abs_count += 1;
            }
        }
        if abs_count == 0 {
            MIN_DBFS
        } else {
            let gamma_r = K_OFFSET + 10.0 * libm::log10(abs_sum / abs_count as f64)
                - REL_GATE_OFFSET_LU;
            let rel_thresh = libm::pow(10.0, (gamma_r - K_OFFSET) / 10.0);
            let mut rel_sum = 0.0f64;
            let mut rel_count = 0usize;
            for w in sub_blocks.windows(4) {
                let z = 0.25 * (w[0] + w[1] + w[2] + w[3]);
                if z > abs_thresh && z > rel_thresh {
                    rel_sum += z;
                    rel_count += 1;
                }
            }
            if rel_count == 0 {
                MIN_DBFS
            } else {
                mean_sq_to_lufs(rel_sum / rel_count as f64)
            }
        }
    };

    Ok(LoudnessReadout {
        integrated_lufs,
        momentary_max_lufs: momentary_max,
        short_term_max_lufs: short_term_max,
        true_peak_dbtp: tp_dbtp,
        true_peak_linear: tp_lin,
    })
}

/// Removes 50 Hz / 60 Hz AC mains hum and up to its 6th harmonic from an [`AudioClip`].
pub fn clean_hum(clip: &mut AudioClip, hum_hz: f32) -> Result<(), SerenadeError> {
    if !hum_hz.is_finite() || !(20.0..=120.0).contains(&hum_hz) {
        return Err(SerenadeError::InvalidParameter(
            "hum_hz must be finite and within 20..=120 Hz",
        ));
    }
    let (ch, sr, total_frames) = validate_clip(clip)?;
    if total_frames == 0 {
        return Ok(());
    }

    let sr_f64 = sr as f64;
    let mut notches = Vec::new();
    for h in 1..=6 {
        let f = (hum_hz as f64) * (h as f64);
        if f < sr_f64 * 0.45 {
            notches.push(Biquad::notch(f, 25.0, sr_f64));
        }
    }

    for frame_idx in 0..total_frames {
        let base = frame_idx * ch;
        for c in 0..ch {
            let mut s = sanitize(clip.samples[base + c]) as f64;
            for n in &mut notches {
                s = n.process_ch(c, s);
            }
            clip.samples[base + c] = s as f32;
        }
    }
    Ok(())
}

/// Suppresses stationary background noise / room hiss across 8 phase-aligned sub-bands
/// (`reduction_db` in `0.0..=36.0`).
pub fn suppress_background_noise(
    clip: &mut AudioClip,
    reduction_db: f32,
) -> Result<(), SerenadeError> {
    if !reduction_db.is_finite() || !(0.0..=36.0).contains(&reduction_db) {
        return Err(SerenadeError::InvalidParameter(
            "reduction_db must be within 0..=36 dB",
        ));
    }
    let (ch, sr, total_frames) = validate_clip(clip)?;
    if total_frames == 0 || reduction_db <= 0.0 {
        return Ok(());
    }

    let sr_f64 = sr as f64;
    // Scale cutoffs gracefully if sample_rate < 44.1 kHz
    let scale = (sr_f64 / 48_000.0).min(1.0);
    let f1500 = 1500.0 * scale;
    let f400 = 400.0 * scale;
    let f5000 = 5000.0 * scale;
    let f180 = 180.0 * scale;
    let f800 = 800.0 * scale;
    let f2800 = 2800.0 * scale;
    let f8500 = 8500.0 * scale;

    struct Bank8 {
        s1500: Lr2,
        s400: Lr2,
        s5000: Lr2,
        s180: Lr2,
        s800: Lr2,
        s2800: Lr2,
        s8500: Lr2,
        ap_l_5000: Allpass1,
        ap_l_2800: Allpass1,
        ap_l_8500: Allpass1,
        ap_h_400: Allpass1,
        ap_h_180: Allpass1,
        ap_h_800: Allpass1,
        ap_ll_800: Allpass1,
        ap_lh_180: Allpass1,
        ap_hl_8500: Allpass1,
        ap_hh_2800: Allpass1,
    }
    impl Bank8 {
        fn new(
            f1500: f64,
            f400: f64,
            f5000: f64,
            f180: f64,
            f800: f64,
            f2800: f64,
            f8500: f64,
            sr: f64,
        ) -> Self {
            Self {
                s1500: Lr2::new(f1500, sr),
                s400: Lr2::new(f400, sr),
                s5000: Lr2::new(f5000, sr),
                s180: Lr2::new(f180, sr),
                s800: Lr2::new(f800, sr),
                s2800: Lr2::new(f2800, sr),
                s8500: Lr2::new(f8500, sr),
                ap_l_5000: Allpass1::new(f5000, sr),
                ap_l_2800: Allpass1::new(f2800, sr),
                ap_l_8500: Allpass1::new(f8500, sr),
                ap_h_400: Allpass1::new(f400, sr),
                ap_h_180: Allpass1::new(f180, sr),
                ap_h_800: Allpass1::new(f800, sr),
                ap_ll_800: Allpass1::new(f800, sr),
                ap_lh_180: Allpass1::new(f180, sr),
                ap_hl_8500: Allpass1::new(f8500, sr),
                ap_hh_2800: Allpass1::new(f2800, sr),
            }
        }
        fn split(&mut self, frame: [f32; 2], ch: usize) -> [[f32; 2]; 8] {
            let (l, h) = self.s1500.split(frame, ch);
            let l = self
                .ap_l_8500
                .process_frame(self.ap_l_2800.process_frame(self.ap_l_5000.process_frame(l, ch), ch), ch);
            let h = self
                .ap_h_800
                .process_frame(self.ap_h_180.process_frame(self.ap_h_400.process_frame(h, ch), ch), ch);
            let (ll, lh) = self.s400.split(l, ch);
            let (hl, hh) = self.s5000.split(h, ch);
            let ll = self.ap_ll_800.process_frame(ll, ch);
            let lh = self.ap_lh_180.process_frame(lh, ch);
            let hl = self.ap_hl_8500.process_frame(hl, ch);
            let hh = self.ap_hh_2800.process_frame(hh, ch);
            let (b0, b1) = self.s180.split(ll, ch);
            let (b2, b3) = self.s800.split(lh, ch);
            let (b4, b5) = self.s2800.split(hl, ch);
            let (b6, b7) = self.s8500.split(hh, ch);
            [b0, b1, b2, b3, b4, b5, b6, b7]
        }
        fn reset(&mut self) {
            self.s1500.reset();
            self.s400.reset();
            self.s5000.reset();
            self.s180.reset();
            self.s800.reset();
            self.s2800.reset();
            self.s8500.reset();
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
    }

    let mut bank = Bank8::new(f1500, f400, f5000, f180, f800, f2800, f8500, sr_f64);

    // Pass 1: Estimate noise floor per band from 20 ms chunks if dynamic range exists
    let win_frames = ((sr as usize) / 50).max(64);
    let num_wins = total_frames / win_frames;
    let mut seeded_floor = [1e-4f32; 8];
    let mut use_seeded = false;

    if num_wins >= 4 {
        let mut win_bands: [Vec<f32>; 8] = core::array::from_fn(|_| Vec::new());
        let mut total_win_rms = Vec::new();
        for wb in &mut win_bands {
            wb.try_reserve(num_wins)
                .map_err(|_| SerenadeError::AllocationFailed)?;
        }
        total_win_rms
            .try_reserve(num_wins)
            .map_err(|_| SerenadeError::AllocationFailed)?;

        for w in 0..num_wins {
            let mut sum_sq = [0.0f64; 8];
            let mut tot_sq = 0.0f64;
            for f in 0..win_frames {
                let base = (w * win_frames + f) * ch;
                let frame = [
                    sanitize(clip.samples[base]),
                    if ch > 1 { sanitize(clip.samples[base + 1]) } else { 0.0 },
                ];
                let e = (frame[0] as f64) * (frame[0] as f64) + (frame[1] as f64) * (frame[1] as f64);
                tot_sq += e / (ch as f64);
                let bands = bank.split(frame, ch);
                for (b, s) in bands.iter().enumerate() {
                    let be = ((s[0] as f64) * (s[0] as f64) + (s[1] as f64) * (s[1] as f64))
                        / (ch as f64);
                    sum_sq[b] += be;
                }
            }
            let rms_tot = libm::sqrt(tot_sq / win_frames as f64) as f32;
            total_win_rms.push(rms_tot);
            for b in 0..8 {
                win_bands[b].push(libm::sqrt(sum_sq[b] / win_frames as f64) as f32);
            }
        }

        total_win_rms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
        let p10_tot = total_win_rms[num_wins / 10];
        let p90_tot = total_win_rms[(num_wins * 9) / 10];
        if p10_tot > 1e-6 && p10_tot < p90_tot * 0.45 {
            use_seeded = true;
            for b in 0..8 {
                win_bands[b].sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
                // 15th percentile captures room-noise floor accurately without picking dead zero outliers
                seeded_floor[b] = win_bands[b][(num_wins * 15) / 100].max(1e-6);
            }
        }
        bank.reset();
    }

    // Pass 2: Smooth sub-band Wiener noise suppression
    let sr_f32 = sr as f32;
    let min_gain = db_to_lin(-reduction_db);
    let sensitivity = 1.6f32;
    let env_att = time_coeff(4.0, sr_f32);
    let env_rel = time_coeff(55.0, sr_f32);
    let floor_dn = time_coeff(35.0, sr_f32);
    let floor_up = time_coeff(2500.0, sr_f32);
    let gain_att = time_coeff(8.0, sr_f32);
    let gain_rel = time_coeff(85.0, sr_f32);

    let mut band_env = if use_seeded { seeded_floor } else { [0.0; 8] };
    let mut noise_floor = seeded_floor;
    let mut band_gain = [1.0f32; 8];
    let warm_up = (sr_f32 * 0.015) as usize;

    for frame_idx in 0..total_frames {
        let base = frame_idx * ch;
        let frame = [
            sanitize(clip.samples[base]),
            if ch > 1 { sanitize(clip.samples[base + 1]) } else { 0.0 },
        ];
        let bands = bank.split(frame, ch);
        let mut out = [0.0f32; 2];

        for b in 0..8 {
            let energy = (bands[b][0] * bands[b][0] + bands[b][1] * bands[b][1]) / (ch as f32);
            let level = libm::sqrtf(energy);
            let ec = if level > band_env[b] { env_att } else { env_rel };
            band_env[b] = ec * band_env[b] + (1.0 - ec) * level;

            if !use_seeded {
                if frame_idx == warm_up / 2 {
                    noise_floor[b] = band_env[b].max(1e-6);
                } else if frame_idx > warm_up {
                    let fc = if band_env[b] < noise_floor[b] {
                        floor_dn
                    } else {
                        floor_up
                    };
                    noise_floor[b] =
                        (fc * noise_floor[b] + (1.0 - fc) * band_env[b]).max(1e-6);
                }
            }

            let target_gain = if !use_seeded && frame_idx <= warm_up {
                1.0
            } else {
                let env_sq = band_env[b] * band_env[b];
                let thresh = noise_floor[b] * sensitivity;
                let thresh_sq = thresh * thresh;
                if env_sq <= 1e-12 {
                    min_gain
                } else {
                    let excess = (env_sq - thresh_sq).max(0.0) / (env_sq + 1e-12);
                    min_gain + (1.0 - min_gain) * libm::sqrtf(excess)
                }
            };

            let gc = if target_gain > band_gain[b] {
                gain_att
            } else {
                gain_rel
            };
            band_gain[b] = gc * band_gain[b] + (1.0 - gc) * target_gain;

            for c in 0..ch {
                out[c] += bands[b][c] * band_gain[b];
            }
        }
        for c in 0..ch {
            clip.samples[base + c] = out[c];
        }
    }

    Ok(())
}

/// Split-band `4.5–10 kHz` de-esser removing harsh `"s"`/`"sh"` sibilance from an [`AudioClip`].
pub fn de_ess(
    clip: &mut AudioClip,
    threshold_db: f32,
    reduction_db: f32,
) -> Result<(), SerenadeError> {
    if !threshold_db.is_finite() || !(-60.0..=0.0).contains(&threshold_db) {
        return Err(SerenadeError::InvalidParameter(
            "threshold_db must be within -60..=0 dBFS",
        ));
    }
    if !reduction_db.is_finite() || !(0.0..=30.0).contains(&reduction_db) {
        return Err(SerenadeError::InvalidParameter(
            "reduction_db must be within 0..=30 dB",
        ));
    }
    let (ch, sr, total_frames) = validate_clip(clip)?;
    if total_frames == 0 || reduction_db <= 0.0 {
        return Ok(());
    }

    let sr_f64 = sr as f64;
    let crossover = 4500.0_f64.min(sr_f64 * 0.40);
    let q = 0.70710678;
    let mut lp1 = Biquad::lowpass(crossover, q, sr_f64);
    let mut lp2 = Biquad::lowpass(crossover, q, sr_f64);
    let mut hp1 = Biquad::highpass(crossover, q, sr_f64);
    let mut hp2 = Biquad::highpass(crossover, q, sr_f64);
    let mut sc_lp = Biquad::lowpass(10_000.0_f64.min(sr_f64 * 0.44), q, sr_f64);

    let sr_f32 = sr as f32;
    let thresh_lin = db_to_lin(threshold_db);
    let ratio = 5.5f32;
    let att = time_coeff(0.8, sr_f32);
    let rel = time_coeff(45.0, sr_f32);
    let mut env = 0.0f32;

    for frame_idx in 0..total_frames {
        let base = frame_idx * ch;
        let frame = [
            sanitize(clip.samples[base]),
            if ch > 1 { sanitize(clip.samples[base + 1]) } else { 0.0 },
        ];
        let low = lp2.process_frame(lp1.process_frame(frame, ch), ch);
        let high = hp2.process_frame(hp1.process_frame(frame, ch), ch);
        let sib = sc_lp.process_frame(high, ch);

        let peak = libm::fabsf(sib[0]).max(if ch > 1 { libm::fabsf(sib[1]) } else { 0.0 });
        let c = if peak > env { att } else { rel };
        env = c * env + (1.0 - c) * peak;

        let gain = if env > thresh_lin {
            let over_db = (lin_to_db(env) - threshold_db).max(0.0);
            let cut_db = (over_db * (1.0 - 1.0 / ratio)).min(reduction_db);
            db_to_lin(-cut_db)
        } else {
            1.0
        };

        for c in 0..ch {
            clip.samples[base + c] = low[c] + high[c] * gain;
        }
    }

    Ok(())
}

/// Normalizes an [`AudioClip`] to `target_lufs` (e.g. `-16.0` LUFS for podcasts) with a
/// 4x oversampled True-Peak brickwall limiter at `-1.0 dBTP`.
pub fn normalize_podcast_lufs(
    clip: &mut AudioClip,
    target_lufs: f32,
) -> Result<LoudnessReadout, SerenadeError> {
    if !target_lufs.is_finite() || !(-60.0..=-1.0).contains(&target_lufs) {
        return Err(SerenadeError::InvalidParameter(
            "target_lufs must be within -60..=-1 LUFS",
        ));
    }
    let (ch, sr, total_frames) = validate_clip(clip)?;
    let before = measure_loudness(clip)?;
    if total_frames == 0 || before.integrated_lufs <= MIN_DBFS + 1.0 || before.true_peak_linear <= 1e-7 {
        return Ok(before);
    }

    let gain_db = (target_lufs - before.integrated_lufs).clamp(-48.0, 48.0);
    let gain_lin = db_to_lin(gain_db);
    let ceiling_lin = db_to_lin(TRUE_PEAK_CEILING_DBTP);

    for s in &mut clip.samples {
        *s = sanitize(*s) * gain_lin;
    }

    apply_clip_true_peak_brickwall(clip, ch, sr, total_frames, ceiling_lin)?;
    let mut after = measure_loudness(clip)?;

    // Up to 3 bounded makeup passes if limiting attenuated peaks below target loudness
    for _ in 0..3 {
        let residual = target_lufs - after.integrated_lufs;
        if residual <= 0.2 || after.integrated_lufs <= MIN_DBFS + 1.0 {
            break;
        }
        let makeup = db_to_lin(residual.min(6.0));
        for s in &mut clip.samples {
            *s *= makeup;
        }
        apply_clip_true_peak_brickwall(clip, ch, sr, total_frames, ceiling_lin)?;
        let next = measure_loudness(clip)?;
        if next.integrated_lufs <= after.integrated_lufs + 0.05 {
            after = next;
            break;
        }
        after = next;
    }

    Ok(after)
}

fn apply_clip_true_peak_brickwall(
    clip: &mut AudioClip,
    ch: usize,
    sr: u32,
    total_frames: usize,
    ceiling_lin: f32,
) -> Result<(), SerenadeError> {
    if total_frames == 0 {
        return Ok(());
    }
    let mut tp = TruePeak4x::new();
    let mut req_gain = Vec::new();
    req_gain
        .try_reserve_exact(total_frames)
        .map_err(|_| SerenadeError::AllocationFailed)?;

    let mut engaged = false;
    let lead = 5.min(total_frames);
    for idx in 0..lead {
        let base = idx * ch;
        let f = [
            clip.samples[base],
            if ch > 1 { clip.samples[base + 1] } else { 0.0 },
        ];
        tp.process_frame(f, ch);
    }
    for i in 0..total_frames {
        let step_tp = if i + 5 < total_frames {
            let base = (i + 5) * ch;
            let f = [
                clip.samples[base],
                if ch > 1 { clip.samples[base + 1] } else { 0.0 },
            ];
            tp.process_frame(f, ch)
        } else {
            tp.process_frame([0.0, 0.0], ch)
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
        let max_tp = tp.max_peak;
        if max_tp > ceiling_lin && max_tp > 1e-9 {
            let trim = (ceiling_lin * 0.999) / max_tp;
            for s in &mut clip.samples {
                *s *= trim;
            }
        }
        return Ok(());
    }

    let attack_frames = ((sr as f32 * 0.0015) as usize).max(16);
    let attack_step = 1.0f32 / (attack_frames as f32);
    for i in (0..total_frames.saturating_sub(1)).rev() {
        let next = req_gain[i + 1] + attack_step;
        if next < req_gain[i] {
            req_gain[i] = next;
        }
    }

    let rel_coeff = libm::expf(-1.0 / (0.055 * sr as f32));
    let mut gr = 0.0f32;
    for i in 0..total_frames {
        let needed_gr = (1.0 - req_gain[i]).max(0.0);
        gr = (gr * rel_coeff).max(needed_gr);
        let g = (1.0 - gr).clamp(0.0, 1.0);
        let base = i * ch;
        for c in 0..ch {
            clip.samples[base + c] *= g;
        }
    }

    let verify = measure_loudness(clip)?;
    if verify.true_peak_linear > ceiling_lin && verify.true_peak_linear > 1e-9 {
        let trim = (ceiling_lin * 0.998) / verify.true_peak_linear;
        for s in &mut clip.samples {
            *s *= trim;
        }
    }

    Ok(())
}

/// 1-click OmaAudio-grade studio podcast voice polish:
/// DC offset removal -> 6-harmonic hum removal -> 8-band background noise suppression ->
/// 4.5–10 kHz de-esser -> `-16.0 LUFS` podcast loudness normalization (`-1.0 dBTP` ceiling).
pub fn polish_podcast_voice(
    clip: &mut AudioClip,
    hum_hz: f32,
) -> Result<LoudnessReadout, SerenadeError> {
    clip.remove_dc_offset();
    clean_hum(clip, hum_hz)?;
    suppress_background_noise(clip, 14.0)?;
    de_ess(clip, -24.0, 10.0)?;
    normalize_podcast_lufs(clip, PODCAST_TARGET_LUFS)
}

impl AudioClip {
    /// Measures ITU-R BS.1770-4 / EBU R128 LUFS and 4x oversampled True-Peak dBTP on this clip.
    pub fn loudness(&self) -> Result<LoudnessReadout, SerenadeError> {
        measure_loudness(self)
    }

    /// Removes 50/60 Hz mains hum and up to its 6th harmonic in-place.
    pub fn clean_hum(&mut self, hum_hz: f32) -> Result<(), SerenadeError> {
        clean_hum(self, hum_hz)
    }

    /// Suppresses stationary background noise / room hiss in-place (`0..=36 dB`).
    pub fn suppress_background_noise(&mut self, reduction_db: f32) -> Result<(), SerenadeError> {
        suppress_background_noise(self, reduction_db)
    }

    /// Reduces harsh `4.5–10 kHz` sibilance in-place.
    pub fn de_ess(&mut self, threshold_db: f32, reduction_db: f32) -> Result<(), SerenadeError> {
        de_ess(self, threshold_db, reduction_db)
    }

    /// Normalizes this clip to `target_lufs` (typically `-16.0` LUFS) with `-1.0 dBTP` brickwall ceiling.
    pub fn normalize_podcast_lufs(
        &mut self,
        target_lufs: f32,
    ) -> Result<LoudnessReadout, SerenadeError> {
        normalize_podcast_lufs(self, target_lufs)
    }

    /// Applies 1-click OmaAudio-grade studio podcast voice post-processing and returns final LUFS/dBTP.
    pub fn polish_podcast_voice(&mut self, hum_hz: f32) -> Result<LoudnessReadout, SerenadeError> {
        polish_podcast_voice(self, hum_hz)
    }
}
