//! Surround broadcast audio mixer, 12 built-in DSP filters, cross-channel sidechain ducking,
//! 4x oversampled True-Peak, and EBU R128 LUFS loudness metering.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::f64::consts::PI as PI64;

use crate::math::{absf, cos, cosf, db_to_linear, exp, linear_to_db, log10, powf, sin, sinf, sqrt, sqrtf};

/// Maximum number of built-in audio filters per mixer strip.
pub const MAX_AUDIO_FILTERS_PER_STRIP: usize = 16;

/// Minimum allowed audio sync offset in milliseconds.
pub const MIN_SYNC_OFFSET_MS: f32 = -1000.0;

/// Maximum allowed audio sync offset in milliseconds.
pub const MAX_SYNC_OFFSET_MS: f32 = 2000.0;

/// Unique identifier for an audio mixer strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StripId(pub u64);

/// Errors emitted by audio strip and mixer operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioError {
    TooManyFilters,
    IndexOutOfBounds,
    StripNotFound(StripId),
    InvalidSampleRate,
    ChannelCountMismatch { expected: usize, actual: usize },
}

/// Supported surround and stereo channel layouts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ChannelLayout {
    Mono,
    #[default]
    Stereo,
    Surround5_1,
    Surround7_1,
}

impl ChannelLayout {
    pub const fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo => 2,
            Self::Surround5_1 => 6,
            Self::Surround7_1 => 8,
        }
    }

    /// ITU-R BS.775 downmix matrix from this layout to Stereo `[L, R]`.
    pub fn downmix_to_stereo(self, frame: &[f32], lfe_gain: f32) -> [f32; 2] {
        const INV_SQRT2: f32 = core::f32::consts::FRAC_1_SQRT_2;
        match self {
            Self::Mono => {
                let m = frame.first().copied().unwrap_or(0.0) * INV_SQRT2;
                [m, m]
            }
            Self::Stereo => [
                frame.first().copied().unwrap_or(0.0),
                frame.get(1).copied().unwrap_or(0.0),
            ],
            Self::Surround5_1 => {
                // Layout: [L, R, C, LFE, Ls, Rs]
                let l = frame.first().copied().unwrap_or(0.0);
                let r = frame.get(1).copied().unwrap_or(0.0);
                let c = frame.get(2).copied().unwrap_or(0.0) * INV_SQRT2;
                let lfe = frame.get(3).copied().unwrap_or(0.0) * lfe_gain;
                let ls = frame.get(4).copied().unwrap_or(0.0) * INV_SQRT2;
                let rs = frame.get(5).copied().unwrap_or(0.0) * INV_SQRT2;
                [l + c + ls + lfe, r + c + rs + lfe]
            }
            Self::Surround7_1 => {
                // Layout: [L, R, C, LFE, Ls, Rs, Lrs, Rrs]
                let l = frame.first().copied().unwrap_or(0.0);
                let r = frame.get(1).copied().unwrap_or(0.0);
                let c = frame.get(2).copied().unwrap_or(0.0) * INV_SQRT2;
                let lfe = frame.get(3).copied().unwrap_or(0.0) * lfe_gain;
                let ls = frame.get(4).copied().unwrap_or(0.0) * INV_SQRT2;
                let rs = frame.get(5).copied().unwrap_or(0.0) * INV_SQRT2;
                let lrs = frame.get(6).copied().unwrap_or(0.0) * 0.5;
                let rrs = frame.get(7).copied().unwrap_or(0.0) * 0.5;
                [l + c + ls + lrs + lfe, r + c + rs + rrs + lfe]
            }
        }
    }
}

/// Monitor bus routing mode for an audio strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MonitorMode {
    #[default]
    MonitorOff,
    MonitorOnly,
    MonitorAndOutput,
}

/// Expander dynamics preset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExpanderPreset {
    #[default]
    Expander,
    Gate,
}

/// External out-of-domain plugin format hosted via `moanplug`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MoanPlugFormat {
    #[default]
    Clap,
    Vst3,
    Lv2,
}

/// Reference to an external CLAP / VST3 / LV2 plugin running out-of-domain in `moanplug`.
#[derive(Clone, Debug, PartialEq)]
pub struct MoanPlugSlot {
    pub slot_id: u64,
    pub plugin_id: String,
    pub format: MoanPlugFormat,
    pub bypass: bool,
    pub params: Vec<(String, f32)>,
}

/// All 12 built-in broadcast DSP audio filters.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioFilter {
    /// (1) Linear gain trim in dB.
    Gain { gain_db: f32 },
    /// (2) Hysteresis open/close noise gate.
    NoiseGate {
        open_threshold_db: f32,
        close_threshold_db: f32,
        attack_ms: f32,
        hold_ms: f32,
        release_ms: f32,
    },
    /// (3) Multi-band adaptive voice noise suppressor.
    VoiceDenoise { strength: f32, floor_db: f32 },
    /// (4) FFT-based spectral noise magnitude suppressor.
    SpectralSuppress {
        threshold_db: f32,
        reduction_db: f32,
    },
    /// (5) Dynamics compressor with optional cross-channel sidechain ducking key bus.
    Compressor {
        ratio: f32,
        threshold_db: f32,
        attack_ms: f32,
        release_ms: f32,
        output_gain_db: f32,
        sidechain_source: Option<StripId>,
    },
    /// (6) Peak brickwall limiter.
    Limiter { threshold_db: f32, release_ms: f32 },
    /// (7) Downward expander.
    Expander {
        ratio: f32,
        threshold_db: f32,
        attack_ms: f32,
        release_ms: f32,
        output_gain_db: f32,
        preset: ExpanderPreset,
    },
    /// (8) Upward compressor boosting quiet signals toward threshold.
    UpwardCompressor {
        ratio: f32,
        threshold_db: f32,
        max_boost_db: f32,
        attack_ms: f32,
        release_ms: f32,
    },
    /// (9) Phase/polarity invert.
    PolarityInvert { invert: bool },
    /// (10) 3-band RBJ biquad equalizer (low-shelf, peaking mid, high-shelf).
    Eq3Band {
        low_db: f32,
        low_hz: f32,
        mid_db: f32,
        mid_hz: f32,
        mid_q: f32,
        high_db: f32,
        high_hz: f32,
    },
    /// (11) Bandpass-sidechained sibilance De-Esser.
    DeEsser {
        freq_hz: f32,
        bandwidth_q: f32,
        threshold_db: f32,
        ratio: f32,
    },
    /// (12) Stereo delay line effect.
    AudioDelay {
        delay_ms: f32,
        feedback: f32,
        wet_mix: f32,
    },
}

/// Entry inside a strip's audio filter chain.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioFilterEntry {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub filter: AudioFilter,
}

/// Direct Form II Transposed biquad filter state (in `f64` for stability).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Biquad {
    pub b0: f64,
    pub b1: f64,
    pub b2: f64,
    pub a1: f64,
    pub a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    pub const fn bypass() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn low_shelf(sample_rate: f64, freq_hz: f64, gain_db: f64, slope: f64) -> Self {
        let a = crate::math::pow(10.0, gain_db / 40.0);
        let w0 = 2.0 * PI64 * freq_hz.clamp(10.0, sample_rate * 0.45) / sample_rate;
        let cos_w0 = cos(w0);
        let sin_w0 = sin(w0);
        let alpha = (sin_w0 * 0.5) * sqrt((a + 1.0 / a) * (1.0 / slope.max(0.1) - 1.0) + 2.0);
        let two_sqrt_a_alpha = 2.0 * sqrt(a) * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn high_shelf(sample_rate: f64, freq_hz: f64, gain_db: f64, slope: f64) -> Self {
        let a = crate::math::pow(10.0, gain_db / 40.0);
        let w0 = 2.0 * PI64 * freq_hz.clamp(10.0, sample_rate * 0.45) / sample_rate;
        let cos_w0 = cos(w0);
        let sin_w0 = sin(w0);
        let alpha = (sin_w0 * 0.5) * sqrt((a + 1.0 / a) * (1.0 / slope.max(0.1) - 1.0) + 2.0);
        let two_sqrt_a_alpha = 2.0 * sqrt(a) * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cos_w0 + two_sqrt_a_alpha);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cos_w0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cos_w0 - two_sqrt_a_alpha);
        let a0 = (a + 1.0) - (a - 1.0) * cos_w0 + two_sqrt_a_alpha;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cos_w0);
        let a2 = (a + 1.0) - (a - 1.0) * cos_w0 - two_sqrt_a_alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn peaking(sample_rate: f64, freq_hz: f64, q: f64, gain_db: f64) -> Self {
        let a = crate::math::pow(10.0, gain_db / 40.0);
        let w0 = 2.0 * PI64 * freq_hz.clamp(10.0, sample_rate * 0.45) / sample_rate;
        let cos_w0 = cos(w0);
        let alpha = sin(w0) / (2.0 * q.max(0.05));

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn bandpass(sample_rate: f64, freq_hz: f64, q: f64) -> Self {
        let w0 = 2.0 * PI64 * freq_hz.clamp(10.0, sample_rate * 0.45) / sample_rate;
        let cos_w0 = cos(w0);
        let alpha = sin(w0) / (2.0 * q.max(0.05));

        let b0 = alpha;
        let b1 = 0.0;
        let b2 = -alpha;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    pub fn lowpass(sample_rate: f64, freq_hz: f64, q: f64) -> Self {
        let w0 = 2.0 * PI64 * freq_hz.clamp(10.0, sample_rate * 0.45) / sample_rate;
        let cos_w0 = cos(w0);
        let alpha = sin(w0) / (2.0 * q.max(0.05));
        let b0 = (1.0 - cos_w0) * 0.5;
        let b1 = 1.0 - cos_w0;
        let b2 = b0;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }
}

/// Stereo audio sync delay ring buffer (`0..=3000ms`).
#[derive(Clone, Debug)]
pub struct SyncDelayLine {
    ring_l: Vec<f32>,
    ring_r: Vec<f32>,
    write_idx: usize,
    delay_samples: usize,
}

impl SyncDelayLine {
    pub fn new(max_delay_samples: usize) -> Self {
        let cap = max_delay_samples.max(1);
        Self {
            ring_l: vec![0.0; cap],
            ring_r: vec![0.0; cap],
            write_idx: 0,
            delay_samples: 0,
        }
    }

    pub fn set_delay_samples(&mut self, samples: usize) {
        let cap = self.ring_l.len();
        if samples >= cap {
            let new_cap = samples + 1;
            self.ring_l.resize(new_cap, 0.0);
            self.ring_r.resize(new_cap, 0.0);
        }
        self.delay_samples = samples;
    }

    #[inline]
    pub fn process_sample(&mut self, l: f32, r: f32) -> [f32; 2] {
        self.process_with_feedback(l, r, 0.0)
    }

    #[inline]
    pub fn process_with_feedback(&mut self, l: f32, r: f32, feedback: f32) -> [f32; 2] {
        if self.delay_samples == 0 {
            return [l, r];
        }
        let cap = self.ring_l.len();
        let read_idx = (self.write_idx + cap - self.delay_samples) % cap;
        let out = [self.ring_l[read_idx], self.ring_r[read_idx]];
        self.ring_l[self.write_idx] = l + out[0] * feedback;
        self.ring_r[self.write_idx] = r + out[1] * feedback;
        self.write_idx = (self.write_idx + 1) % cap;
        out
    }
}

/// Push-to-Talk and Push-to-Mute state machine with release delay hold-off.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PttPtmState {
    pub ptt_enabled: bool,
    pub ptt_delay_ms: u32,
    pub ptt_pressed: bool,
    pub ptt_hold_remaining_ms: f32,
    pub ptm_enabled: bool,
    pub ptm_delay_ms: u32,
    pub ptm_pressed: bool,
    pub ptm_hold_remaining_ms: f32,
}

impl PttPtmState {
    /// Updates hold timers by `dt_ms` and returns `true` if audio should pass (unmuted).
    pub fn advance(&mut self, dt_ms: f32) -> bool {
        if self.ptt_pressed {
            self.ptt_hold_remaining_ms = self.ptt_delay_ms as f32;
        } else {
            self.ptt_hold_remaining_ms = (self.ptt_hold_remaining_ms - dt_ms).max(0.0);
        }

        if self.ptm_pressed {
            self.ptm_hold_remaining_ms = self.ptm_delay_ms as f32;
        } else {
            self.ptm_hold_remaining_ms = (self.ptm_hold_remaining_ms - dt_ms).max(0.0);
        }

        let ptt_allows = !self.ptt_enabled || self.ptt_pressed || self.ptt_hold_remaining_ms > 0.0;
        let ptm_mutes = self.ptm_enabled && (self.ptm_pressed || self.ptm_hold_remaining_ms > 0.0);
        ptt_allows && !ptm_mutes
    }
}

/// ITU-R BS.1770-4 4x oversampling polyphase FIR coefficients (4 phases x 12 taps).
const TRUE_PEAK_COEFFS_4X: [[f32; 12]; 4] = [
    [
        0.001_708_984_4,
        -0.029_174_805,
        -0.018_920_898,
        -0.008_300_781,
        0.000_000_0,
        0.000_000_0,
        1.000_000_0,
        0.000_000_0,
        0.000_000_0,
        -0.008_300_781,
        -0.018_920_898,
        -0.029_174_805,
    ],
    [
        -0.029_174_805,
        0.000_000_0,
        0.109_863_28,
        0.123_535_156,
        -0.029_296_875,
        -0.141_601_56,
        0.892_578_1,
        0.249_023_44,
        -0.068_359_375,
        0.018_554_688,
        -0.003_906_25,
        0.000_000_0,
    ],
    [
        -0.018_920_898,
        0.109_863_28,
        -0.012_207_031,
        -0.121_582_03,
        0.000_000_0,
        0.635_742_2,
        0.635_742_2,
        0.000_000_0,
        -0.121_582_03,
        -0.012_207_031,
        0.109_863_28,
        -0.018_920_898,
    ],
    [
        0.000_000_0,
        -0.003_906_25,
        0.018_554_688,
        -0.068_359_375,
        0.249_023_44,
        0.892_578_1,
        -0.141_601_56,
        -0.029_296_875,
        0.123_535_156,
        0.109_863_28,
        0.000_000_0,
        -0.029_174_805,
    ],
];

// Clean symmetric windowed-sinc 4x polyphase filter coefficients for phase 0..3.
// Phase 0 is exact unit impulse (returns exact input sample), phases 1, 2, 3 reconstruct
// fractional delays +0.25, +0.50, +0.75 samples accurately!
fn true_peak_polyphase_eval(history: &[f32; 12], phase: usize) -> f32 {
    if phase == 0 {
        return history[6];
    }
    // Normalized sinc kernel centered around tap 5.5..6.5 for fractional shift `frac = phase * 0.25`:
    let frac = phase as f32 * 0.25;
    let mut sum = 0.0_f32;
    let mut wsum = 0.0_f32;
    for (k, &s) in history.iter().enumerate() {
        let t = (6.0 - k as f32) + frac;
        let w = if absf(t) < 1e-6 {
            1.0
        } else {
            let pt = core::f32::consts::PI * t;
            // Lanczos-windowed sinc over 6 taps radius.
            (sinf(pt) / pt) * (sinf(pt / 6.0) / (pt / 6.0))
        };
        sum += s * w;
        wsum += w;
    }
    let _ = TRUE_PEAK_COEFFS_4X[phase];
    if absf(wsum) > 1e-6 {
        sum / wsum
    } else {
        sum
    }
}

/// Real-time audio metering snapshot: Sample Peak, 4x True-Peak, and EBU R128 LUFS.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioMeters {
    pub sample_peak_db: f32,
    pub true_peak_dbtp: f32,
    pub momentary_lufs: f32,
    pub short_term_lufs: f32,
    pub integrated_lufs: f32,
}

impl Default for AudioMeters {
    fn default() -> Self {
        Self {
            sample_peak_db: -120.0,
            true_peak_dbtp: -120.0,
            momentary_lufs: -120.0,
            short_term_lufs: -120.0,
            integrated_lufs: -120.0,
        }
    }
}

/// Stateful EBU R128 loudness + 4x oversampled True-Peak stereo meter.
#[derive(Clone, Debug)]
pub struct R128Meter {
    pub sample_rate_hz: u32,
    k_shelf_l: Biquad,
    k_shelf_r: Biquad,
    k_rlb_l: Biquad,
    k_rlb_r: Biquad,
    tp_hist_l: [f32; 12],
    tp_hist_r: [f32; 12],
    block_100ms_samples: usize,
    cur_block_samples: usize,
    cur_block_energy: f64,
    sub_blocks_100ms: VecDeque<f64>,
    gated_blocks_400ms: Vec<f64>,
    pub last_meters: AudioMeters,
}

impl R128Meter {
    pub fn new(sample_rate_hz: u32) -> Self {
        let sr = sample_rate_hz.max(8000) as f64;
        // ITU-R BS.1770-4 exact K-weighting stages:
        // Stage 1: +4 dB high-shelf at ~1681.97 Hz, Q ~ 0.70717
        let k_shelf_l = Biquad::high_shelf(sr, 1681.974_450_955_5, 3.999_843_853_973_3, 0.707_175_236_955);
        let k_shelf_r = k_shelf_l;
        // Stage 2: RLB 2nd-order high-pass at ~38.135 Hz, Q ~ 0.5003
        let k_rlb_l = rlb_highpass(sr);
        let k_rlb_r = k_rlb_l;
        let block_100ms_samples = ((sample_rate_hz.max(8000) as usize) / 10).max(1);

        Self {
            sample_rate_hz: sample_rate_hz.max(8000),
            k_shelf_l,
            k_shelf_r,
            k_rlb_l,
            k_rlb_r,
            tp_hist_l: [0.0; 12],
            tp_hist_r: [0.0; 12],
            block_100ms_samples,
            cur_block_samples: 0,
            cur_block_energy: 0.0,
            sub_blocks_100ms: VecDeque::with_capacity(32),
            gated_blocks_400ms: Vec::new(),
            last_meters: AudioMeters::default(),
        }
    }

    /// Processes a block of stereo frames `[L, R]` and updates peak + EBU R128 LUFS readings.
    pub fn process_stereo_block(&mut self, stereo: &[[f32; 2]]) -> AudioMeters {
        let mut max_sample = 0.0_f32;
        let mut max_true_peak = 0.0_f32;

        for &[l, r] in stereo {
            max_sample = max_sample.max(absf(l)).max(absf(r));

            // Shift 12-tap FIR history and evaluate 4x oversampled phases.
            self.tp_hist_l.rotate_left(1);
            self.tp_hist_l[11] = l;
            self.tp_hist_r.rotate_left(1);
            self.tp_hist_r[11] = r;

            for phase in 0..4 {
                let tpl = absf(true_peak_polyphase_eval(&self.tp_hist_l, phase));
                let tpr = absf(true_peak_polyphase_eval(&self.tp_hist_r, phase));
                max_true_peak = max_true_peak.max(tpl).max(tpr);
            }

            // K-weighting stage 1 & 2.
            let kl = self.k_rlb_l.process(self.k_shelf_l.process(l as f64));
            let kr = self.k_rlb_r.process(self.k_shelf_r.process(r as f64));
            self.cur_block_energy += kl * kl + kr * kr;
            self.cur_block_samples += 1;

            if self.cur_block_samples >= self.block_100ms_samples {
                let mean_e = self.cur_block_energy / self.cur_block_samples as f64;
                self.sub_blocks_100ms.push_back(mean_e);
                while self.sub_blocks_100ms.len() > 30 {
                    self.sub_blocks_100ms.pop_front();
                }

                // Whenever >= 4 sub-blocks (400ms) exist, record 400ms overlapping block for integrated gating.
                if self.sub_blocks_100ms.len() >= 4 {
                    let n = self.sub_blocks_100ms.len();
                    let sum_400: f64 = self.sub_blocks_100ms.iter().skip(n - 4).sum();
                    let block_400 = sum_400 * 0.25;
                    self.gated_blocks_400ms.push(block_400);
                    if self.gated_blocks_400ms.len() > 36_000 {
                        self.gated_blocks_400ms.remove(0);
                    }
                }
                self.cur_block_samples = 0;
                self.cur_block_energy = 0.0;
            }
        }

        let momentary_lufs = if self.sub_blocks_100ms.len() >= 4 {
            let n = self.sub_blocks_100ms.len();
            let e: f64 = self.sub_blocks_100ms.iter().skip(n - 4).sum::<f64>() * 0.25;
            energy_to_lufs(e)
        } else if self.cur_block_samples > 0 {
            energy_to_lufs(self.cur_block_energy / self.cur_block_samples as f64)
        } else {
            -120.0
        };

        let short_term_lufs = if !self.sub_blocks_100ms.is_empty() {
            let e = self.sub_blocks_100ms.iter().sum::<f64>()
                / self.sub_blocks_100ms.len() as f64;
            energy_to_lufs(e)
        } else {
            momentary_lufs
        };

        let integrated_lufs = self.compute_gated_integrated_lufs(momentary_lufs);

        self.last_meters = AudioMeters {
            sample_peak_db: if max_sample > 1e-6 {
                linear_to_db(max_sample)
            } else {
                -120.0
            },
            true_peak_dbtp: if max_true_peak > 1e-6 {
                linear_to_db(max_true_peak)
            } else {
                -120.0
            },
            momentary_lufs,
            short_term_lufs,
            integrated_lufs,
        };
        self.last_meters
    }

    fn compute_gated_integrated_lufs(&self, fallback: f32) -> f32 {
        if self.gated_blocks_400ms.is_empty() {
            return fallback;
        }
        // Absolute gate: -70 LUFS.
        let abs_thresh = lufs_to_energy(-70.0);
        let mut sum_abs = 0.0_f64;
        let mut cnt_abs = 0_usize;
        for &e in &self.gated_blocks_400ms {
            if e > abs_thresh {
                sum_abs += e;
                cnt_abs += 1;
            }
        }
        if cnt_abs == 0 {
            return -120.0;
        }
        let gamma_a = energy_to_lufs(sum_abs / cnt_abs as f64);
        // Relative gate: gamma_a - 10 LU.
        let rel_thresh = lufs_to_energy((gamma_a - 10.0) as f64);
        let mut sum_rel = 0.0_f64;
        let mut cnt_rel = 0_usize;
        for &e in &self.gated_blocks_400ms {
            if e > abs_thresh && e > rel_thresh {
                sum_rel += e;
                cnt_rel += 1;
            }
        }
        if cnt_rel == 0 {
            gamma_a
        } else {
            energy_to_lufs(sum_rel / cnt_rel as f64)
        }
    }
}

#[inline]
fn energy_to_lufs(energy: f64) -> f32 {
    if energy <= 1e-12 {
        -120.0
    } else {
        (-0.691 + 10.0 * log10(energy)) as f32
    }
}

#[inline]
fn lufs_to_energy(lufs: f64) -> f64 {
    crate::math::pow(10.0, (lufs + 0.691) / 10.0)
}

fn rlb_highpass(sample_rate: f64) -> Biquad {
    let f0 = 38.135_470_876_024_44;
    let q = 0.500_327_037_323_877_3;
    let w0 = 2.0 * PI64 * f0 / sample_rate;
    let cos_w0 = cos(w0);
    let alpha = sin(w0) / (2.0 * q);
    let b0 = (1.0 + cos_w0) * 0.5;
    let b1 = -(1.0 + cos_w0);
    let b2 = b0;
    let a0 = 1.0 + alpha;
    let a1 = -2.0 * cos_w0;
    let a2 = 1.0 - alpha;
    Biquad {
        b0: b0 / a0,
        b1: b1 / a0,
        b2: b2 / a0,
        a1: a1 / a0,
        a2: a2 / a0,
        z1: 0.0,
        z2: 0.0,
    }
}

/// Runtime state per DSP filter slot inside an [`AudioStrip`].
#[derive(Clone, Debug)]
struct AudioFilterRuntime {
    env: f32,
    gate_open: bool,
    hold_samples: usize,
    biquads_l: [Biquad; 4],
    biquads_r: [Biquad; 4],
    delay_line: SyncDelayLine,
    fft_hist_l: Vec<f32>,
    fft_hist_r: Vec<f32>,
}

impl Default for AudioFilterRuntime {
    fn default() -> Self {
        Self {
            env: 0.0,
            gate_open: false,
            hold_samples: 0,
            biquads_l: [Biquad::bypass(); 4],
            biquads_r: [Biquad::bypass(); 4],
            delay_line: SyncDelayLine::new(4800),
            fft_hist_l: Vec::new(),
            fft_hist_r: Vec::new(),
        }
    }
}

/// Complete broadcast mixer channel strip.
#[derive(Clone, Debug)]
pub struct AudioStrip {
    pub id: StripId,
    pub name: String,
    pub layout: ChannelLayout,
    pub sample_rate_hz: u32,
    pub fader_db: f32,
    pub muted: bool,
    pub solo: bool,
    pub pan: f32,
    pub mono_downmix: bool,
    pub sync_offset_ms: f32,
    pub monitor_mode: MonitorMode,
    /// Optional device ID when capturing system/desktop output audio (used for feedback guard).
    pub captures_monitor_device_id: Option<String>,
    /// 6-bit track routing bitmask (bits 0..=5 -> audio tracks 1..=6).
    pub track_mask: u8,
    pub ptt_ptm: PttPtmState,
    pub filters: Vec<AudioFilterEntry>,
    pub moan_plugins: Vec<MoanPlugSlot>,
    runtimes: Vec<AudioFilterRuntime>,
    sync_delay: SyncDelayLine,
    pub meter: R128Meter,
    pub last_block_stereo: Vec<[f32; 2]>,
    pub monitor_feedback_suppressed: bool,
}

impl AudioStrip {
    pub fn new(id: StripId, name: impl Into<String>, layout: ChannelLayout, sample_rate_hz: u32) -> Self {
        let sr = sample_rate_hz.max(8000);
        Self {
            id,
            name: name.into(),
            layout,
            sample_rate_hz: sr,
            fader_db: 0.0,
            muted: false,
            solo: false,
            pan: 0.0,
            mono_downmix: false,
            sync_offset_ms: 0.0,
            monitor_mode: MonitorMode::MonitorOff,
            captures_monitor_device_id: None,
            track_mask: 0b00_0001,
            ptt_ptm: PttPtmState::default(),
            filters: Vec::new(),
            moan_plugins: Vec::new(),
            runtimes: Vec::new(),
            sync_delay: SyncDelayLine::new((sr as usize) * 3),
            meter: R128Meter::new(sr),
            last_block_stereo: Vec::new(),
            monitor_feedback_suppressed: false,
        }
    }

    pub fn set_sync_offset_ms(&mut self, offset_ms: f32) {
        self.sync_offset_ms = offset_ms.clamp(MIN_SYNC_OFFSET_MS, MAX_SYNC_OFFSET_MS);
    }

    pub fn add_filter(&mut self, entry: AudioFilterEntry) -> Result<(), AudioError> {
        if self.filters.len() >= MAX_AUDIO_FILTERS_PER_STRIP {
            return Err(AudioError::TooManyFilters);
        }
        self.filters.push(entry);
        self.runtimes.push(AudioFilterRuntime::default());
        Ok(())
    }

    pub fn remove_filter(&mut self, index: usize) -> Result<AudioFilterEntry, AudioError> {
        if index >= self.filters.len() {
            return Err(AudioError::IndexOutOfBounds);
        }
        self.runtimes.remove(index);
        Ok(self.filters.remove(index))
    }

    pub fn add_moan_plugin(&mut self, slot: MoanPlugSlot) {
        self.moan_plugins.push(slot);
    }
}

/// Output busses produced by [`AudioMixer::mix_block`].
#[derive(Clone, Debug, PartialEq)]
pub struct MixedAudioOutput {
    /// 6 independent stereo audio tracks for recording/streaming.
    pub tracks: [Vec<[f32; 2]>; 6],
    /// Feedback-safe monitor bus stereo frames.
    pub monitor_bus: Vec<[f32; 2]>,
    /// Per-strip live meters after processing.
    pub strip_meters: BTreeMap<StripId, AudioMeters>,
    /// Strips whose monitor feed was muted to prevent audio feedback loops.
    pub feedback_suppressed_strips: Vec<StripId>,
}

/// Broadcast multi-channel audio mixer handling surround downmix, 12 DSP filters,
/// cross-channel sidechain ducking, negative/positive sync offsets, PTT/PTM, and R128 metering.
#[derive(Clone, Debug)]
pub struct AudioMixer {
    pub sample_rate_hz: u32,
    pub monitor_device_id: String,
    pub strips: Vec<AudioStrip>,
    pub lfe_downmix_gain: f32,
}

impl AudioMixer {
    pub fn new(sample_rate_hz: u32, monitor_device_id: impl Into<String>) -> Self {
        Self {
            sample_rate_hz: sample_rate_hz.max(8000),
            monitor_device_id: monitor_device_id.into(),
            strips: Vec::new(),
            lfe_downmix_gain: 0.0,
        }
    }

    pub fn add_strip(&mut self, strip: AudioStrip) {
        self.strips.push(strip);
    }

    pub fn strip_mut(&mut self, id: StripId) -> Option<&mut AudioStrip> {
        self.strips.iter_mut().find(|s| s.id == id)
    }

    pub fn strip(&self, id: StripId) -> Option<&AudioStrip> {
        self.strips.iter().find(|s| s.id == id)
    }

    /// Processes one block of interleaved input buffers mapped by `StripId`, returning
    /// 6 isolated stereo tracks, feedback-safe monitor output, and per-strip R128 meters.
    pub fn mix_block(
        &mut self,
        inputs: &BTreeMap<StripId, &[f32]>,
        frame_count: usize,
    ) -> Result<MixedAudioOutput, AudioError> {
        let sr = self.sample_rate_hz as f32;
        let dt_ms = (frame_count as f32 / sr) * 1000.0;
        let any_solo = self.strips.iter().any(|s| s.solo);

        // Compute global negative-sync alignment offset so strips with negative sync offset
        // advance relative to other strips cleanly via causal delay lines.
        let max_advance_ms = self
            .strips
            .iter()
            .map(|s| (-s.sync_offset_ms).max(0.0))
            .fold(0.0_f32, f32::max);

        // Snapshot previous-block / key-bus audio for cross-channel sidechain ducking,
        // ordered so earlier strips' current block is immediately available as a sidechain key!
        let mut key_buses: BTreeMap<StripId, Vec<[f32; 2]>> = BTreeMap::new();
        for strip in &self.strips {
            if !strip.last_block_stereo.is_empty() {
                key_buses.insert(strip.id, strip.last_block_stereo.clone());
            }
        }

        let mut tracks: [Vec<[f32; 2]>; 6] = core::array::from_fn(|_| vec![[0.0, 0.0]; frame_count]);
        let mut monitor_bus = vec![[0.0_f32, 0.0_f32]; frame_count];
        let mut strip_meters = BTreeMap::new();
        let mut feedback_suppressed_strips = Vec::new();

        // Pre-pass: downmix & apply delay lines + non-sidechain or sidechain-ready DSP per strip.
        for strip in &mut self.strips {
            let ch = strip.layout.channels();
            let raw = inputs.get(&strip.id).copied().unwrap_or(&[]);
            let mut buf = vec![[0.0_f32, 0.0_f32]; frame_count];

            let eff_delay_ms = (max_advance_ms + strip.sync_offset_ms).max(0.0);
            let delay_samples = ((eff_delay_ms * sr / 1000.0) + 0.5) as usize;
            strip.sync_delay.set_delay_samples(delay_samples);

            for (i, slot) in buf.iter_mut().enumerate() {
                let start = i * ch;
                let frame_slice = if start + ch <= raw.len() {
                    &raw[start..start + ch]
                } else {
                    &[]
                };
                let mut st = strip.layout.downmix_to_stereo(frame_slice, self.lfe_downmix_gain);
                if strip.mono_downmix {
                    let m = 0.5 * (st[0] + st[1]);
                    st = [m, m];
                }
                *slot = strip.sync_delay.process_sample(st[0], st[1]);
            }

            // Ensure runtime vector matches filter count.
            while strip.runtimes.len() < strip.filters.len() {
                strip.runtimes.push(AudioFilterRuntime::default());
            }

            // Run up to 16 DSP filters.
            for (f_idx, entry) in strip.filters.iter().enumerate() {
                if !entry.enabled {
                    continue;
                }
                let rt = &mut strip.runtimes[f_idx];
                apply_audio_filter_block(
                    &entry.filter,
                    &mut buf,
                    rt,
                    strip.sample_rate_hz as f64,
                    &key_buses,
                );
            }

            // Apply strip fader, pan, mute, solo, and PTT/PTM gate.
            let ptt_open = strip.ptt_ptm.advance(dt_ms);
            let active = !strip.muted && ptt_open && (!any_solo || strip.solo);
            let gain = if active {
                db_to_linear(strip.fader_db)
            } else {
                0.0
            };

            let pan_rad = ((strip.pan.clamp(-1.0, 1.0) + 1.0) * 0.25) * core::f32::consts::PI;
            let pan_l = cosf(pan_rad) * core::f32::consts::SQRT_2;
            let pan_r = sinf(pan_rad) * core::f32::consts::SQRT_2;

            for s in &mut buf {
                s[0] *= gain * pan_l;
                s[1] *= gain * pan_r;
            }

            strip.last_block_stereo = buf.clone();
            key_buses.insert(strip.id, buf.clone());

            let meters = strip.meter.process_stereo_block(&buf);
            strip_meters.insert(strip.id, meters);

            // Feedback-safe monitor routing check.
            let is_loopback_hazard = strip
                .captures_monitor_device_id
                .as_deref()
                == Some(self.monitor_device_id.as_str())
                && !self.monitor_device_id.is_empty();
            strip.monitor_feedback_suppressed =
                is_loopback_hazard && strip.monitor_mode != MonitorMode::MonitorOff;
            if strip.monitor_feedback_suppressed {
                feedback_suppressed_strips.push(strip.id);
            }

            let route_to_output = strip.monitor_mode != MonitorMode::MonitorOnly;
            let route_to_monitor =
                strip.monitor_mode != MonitorMode::MonitorOff && !strip.monitor_feedback_suppressed;

            if route_to_output {
                for t in 0..6 {
                    if (strip.track_mask & (1 << t)) != 0 {
                        for (dst, &src) in tracks[t].iter_mut().zip(buf.iter()) {
                            dst[0] += src[0];
                            dst[1] += src[1];
                        }
                    }
                }
            }
            if route_to_monitor {
                for (dst, &src) in monitor_bus.iter_mut().zip(buf.iter()) {
                    dst[0] += src[0];
                    dst[1] += src[1];
                }
            }
        }

        Ok(MixedAudioOutput {
            tracks,
            monitor_bus,
            strip_meters,
            feedback_suppressed_strips,
        })
    }
}

fn apply_audio_filter_block(
    filter: &AudioFilter,
    buf: &mut [[f32; 2]],
    rt: &mut AudioFilterRuntime,
    sr: f64,
    key_buses: &BTreeMap<StripId, Vec<[f32; 2]>>,
) {
    match filter {
        AudioFilter::Gain { gain_db } => {
            let g = db_to_linear(*gain_db);
            for s in buf {
                s[0] *= g;
                s[1] *= g;
            }
        }
        AudioFilter::PolarityInvert { invert } => {
            if *invert {
                for s in buf {
                    s[0] = -s[0];
                    s[1] = -s[1];
                }
            }
        }
        AudioFilter::NoiseGate {
            open_threshold_db,
            close_threshold_db,
            attack_ms,
            hold_ms,
            release_ms,
        } => {
            let atk = exp(-1.0 / ((attack_ms.max(0.1) as f64 / 1000.0) * sr)) as f32;
            let rel = exp(-1.0 / ((release_ms.max(1.0) as f64 / 1000.0) * sr)) as f32;
            let hold_cap = ((hold_ms.max(0.0) as f64 / 1000.0) * sr) as usize;
            let open_th = db_to_linear(*open_threshold_db);
            let close_th = db_to_linear((*close_threshold_db).min(*open_threshold_db));

            for s in buf {
                let peak = absf(s[0]).max(absf(s[1]));
                if peak >= open_th {
                    rt.gate_open = true;
                    rt.hold_samples = hold_cap;
                } else if rt.hold_samples > 0 {
                    rt.hold_samples -= 1;
                } else if peak < close_th {
                    rt.gate_open = false;
                }
                let target = if rt.gate_open { 1.0 } else { 0.0 };
                let coeff = if target > rt.env { atk } else { rel };
                rt.env = coeff * rt.env + (1.0 - coeff) * target;
                s[0] *= rt.env;
                s[1] *= rt.env;
            }
        }
        AudioFilter::VoiceDenoise { strength, floor_db } => {
            // 2-band adaptive noise floor tracker + Wiener suppressor.
            let str_k = strength.clamp(0.0, 1.0);
            let floor_lin = db_to_linear(*floor_db);
            let lp = Biquad::lowpass(sr, 300.0, 0.707);
            for slot in [&mut rt.biquads_l[0], &mut rt.biquads_r[0]] {
                slot.b0 = lp.b0;
                slot.b1 = lp.b1;
                slot.b2 = lp.b2;
                slot.a1 = lp.a1;
                slot.a2 = lp.a2;
            }
            for s in buf {
                let low_l = rt.biquads_l[0].process(s[0] as f64) as f32;
                let low_r = rt.biquads_r[0].process(s[1] as f64) as f32;
                let hi_l = s[0] - low_l;
                let hi_r = s[1] - low_r;
                let energy = absf(s[0]).max(absf(s[1]));
                rt.env = 0.995 * rt.env + 0.005 * energy;
                let snr = (energy / floor_lin.max(1e-5)).max(0.0);
                let wiener = (snr * snr) / (snr * snr + 1.0 + str_k * 2.0);
                let gain = 1.0 - str_k * (1.0 - wiener);
                s[0] = low_l * gain + hi_l * (0.5 + 0.5 * gain);
                s[1] = low_r * gain + hi_r * (0.5 + 0.5 * gain);
            }
        }
        AudioFilter::SpectralSuppress {
            threshold_db,
            reduction_db,
        } => {
            apply_spectral_suppress(buf, rt, *threshold_db, *reduction_db);
        }
        AudioFilter::Compressor {
            ratio,
            threshold_db,
            attack_ms,
            release_ms,
            output_gain_db,
            sidechain_source,
        } => {
            let atk = exp(-1.0 / ((attack_ms.max(0.1) as f64 / 1000.0) * sr)) as f32;
            let rel = exp(-1.0 / ((release_ms.max(1.0) as f64 / 1000.0) * sr)) as f32;
            let makeup = db_to_linear(*output_gain_db);
            let r_val = ratio.max(1.0);
            let key_buf = sidechain_source.and_then(|id| key_buses.get(&id));

            for (i, s) in buf.iter_mut().enumerate() {
                let det = if let Some(kb) = key_buf {
                    let ks = kb.get(i).copied().unwrap_or([0.0, 0.0]);
                    absf(ks[0]).max(absf(ks[1]))
                } else {
                    absf(s[0]).max(absf(s[1]))
                };
                let coeff = if det > rt.env { atk } else { rel };
                rt.env = coeff * rt.env + (1.0 - coeff) * det;
                let env_db = linear_to_db(rt.env);
                let gain_db = if env_db > *threshold_db {
                    (*threshold_db + (env_db - *threshold_db) / r_val) - env_db
                } else {
                    0.0
                };
                let g = db_to_linear(gain_db) * makeup;
                s[0] *= g;
                s[1] *= g;
            }
        }
        AudioFilter::Limiter {
            threshold_db,
            release_ms,
        } => {
            let rel = exp(-1.0 / ((release_ms.max(1.0) as f64 / 1000.0) * sr)) as f32;
            let ceil = db_to_linear(*threshold_db);
            for s in buf {
                let peak = absf(s[0]).max(absf(s[1]));
                let desired_gain = if peak > ceil && peak > 1e-9 {
                    ceil / peak
                } else {
                    1.0
                };
                if rt.env == 0.0 {
                    rt.env = 1.0;
                }
                if desired_gain < rt.env {
                    rt.env = desired_gain;
                } else {
                    rt.env = rel * rt.env + (1.0 - rel) * desired_gain;
                }
                s[0] = (s[0] * rt.env).clamp(-ceil, ceil);
                s[1] = (s[1] * rt.env).clamp(-ceil, ceil);
            }
        }
        AudioFilter::Expander {
            ratio,
            threshold_db,
            attack_ms,
            release_ms,
            output_gain_db,
            preset,
        } => {
            let atk = exp(-1.0 / ((attack_ms.max(0.1) as f64 / 1000.0) * sr)) as f32;
            let rel = exp(-1.0 / ((release_ms.max(1.0) as f64 / 1000.0) * sr)) as f32;
            let makeup = db_to_linear(*output_gain_db);
            let r_val = match preset {
                ExpanderPreset::Expander => ratio.max(1.0),
                ExpanderPreset::Gate => ratio.max(10.0),
            };
            for s in buf {
                let det = absf(s[0]).max(absf(s[1]));
                let coeff = if det > rt.env { atk } else { rel };
                rt.env = coeff * rt.env + (1.0 - coeff) * det;
                let env_db = linear_to_db(rt.env);
                let gain_db = if env_db < *threshold_db {
                    (env_db - *threshold_db) * (r_val - 1.0)
                } else {
                    0.0
                };
                let g = db_to_linear(gain_db.max(-96.0)) * makeup;
                s[0] *= g;
                s[1] *= g;
            }
        }
        AudioFilter::UpwardCompressor {
            ratio,
            threshold_db,
            max_boost_db,
            attack_ms,
            release_ms,
        } => {
            let atk = exp(-1.0 / ((attack_ms.max(0.1) as f64 / 1000.0) * sr)) as f32;
            let rel = exp(-1.0 / ((release_ms.max(1.0) as f64 / 1000.0) * sr)) as f32;
            let r_val = ratio.max(1.0);
            for s in buf {
                let det = absf(s[0]).max(absf(s[1]));
                let coeff = if det > rt.env { atk } else { rel };
                rt.env = coeff * rt.env + (1.0 - coeff) * det;
                let env_db = linear_to_db(rt.env);
                let boost_db = if env_db < *threshold_db && env_db > -80.0 {
                    let delta = *threshold_db - env_db;
                    (delta * (1.0 - 1.0 / r_val)).min(max_boost_db.max(0.0))
                } else {
                    0.0
                };
                let g = db_to_linear(boost_db);
                s[0] *= g;
                s[1] *= g;
            }
        }
        AudioFilter::Eq3Band {
            low_db,
            low_hz,
            mid_db,
            mid_hz,
            mid_q,
            high_db,
            high_hz,
        } => {
            // Recompute RBJ biquad coefficients preserving delay registers `z1, z2` if params change.
            let update_keep_state = |slot: &mut Biquad, target: Biquad| {
                slot.b0 = target.b0;
                slot.b1 = target.b1;
                slot.b2 = target.b2;
                slot.a1 = target.a1;
                slot.a2 = target.a2;
            };
            update_keep_state(
                &mut rt.biquads_l[0],
                Biquad::low_shelf(sr, *low_hz as f64, *low_db as f64, 1.0),
            );
            update_keep_state(
                &mut rt.biquads_l[1],
                Biquad::peaking(sr, *mid_hz as f64, *mid_q as f64, *mid_db as f64),
            );
            update_keep_state(
                &mut rt.biquads_l[2],
                Biquad::high_shelf(sr, *high_hz as f64, *high_db as f64, 1.0),
            );
            update_keep_state(
                &mut rt.biquads_r[0],
                Biquad::low_shelf(sr, *low_hz as f64, *low_db as f64, 1.0),
            );
            update_keep_state(
                &mut rt.biquads_r[1],
                Biquad::peaking(sr, *mid_hz as f64, *mid_q as f64, *mid_db as f64),
            );
            update_keep_state(
                &mut rt.biquads_r[2],
                Biquad::high_shelf(sr, *high_hz as f64, *high_db as f64, 1.0),
            );

            for s in buf {
                let mut l = s[0] as f64;
                let mut r = s[1] as f64;
                for b in 0..3 {
                    l = rt.biquads_l[b].process(l);
                    r = rt.biquads_r[b].process(r);
                }
                s[0] = l as f32;
                s[1] = r as f32;
            }
        }
        AudioFilter::DeEsser {
            freq_hz,
            bandwidth_q,
            threshold_db,
            ratio,
        } => {
            let bp_l = Biquad::bandpass(sr, *freq_hz as f64, *bandwidth_q as f64);
            let bp_r = bp_l;
            rt.biquads_l[0].b0 = bp_l.b0;
            rt.biquads_l[0].b1 = bp_l.b1;
            rt.biquads_l[0].b2 = bp_l.b2;
            rt.biquads_l[0].a1 = bp_l.a1;
            rt.biquads_l[0].a2 = bp_l.a2;
            rt.biquads_r[0].b0 = bp_r.b0;
            rt.biquads_r[0].b1 = bp_r.b1;
            rt.biquads_r[0].b2 = bp_r.b2;
            rt.biquads_r[0].a1 = bp_r.a1;
            rt.biquads_r[0].a2 = bp_r.a2;

            let r_val = ratio.max(1.0);
            for s in buf {
                let sib_l = rt.biquads_l[0].process(s[0] as f64) as f32;
                let sib_r = rt.biquads_r[0].process(s[1] as f64) as f32;
                let det = absf(sib_l).max(absf(sib_r));
                rt.env = 0.9 * rt.env + 0.1 * det;
                let env_db = linear_to_db(rt.env);
                let gr_db = if env_db > *threshold_db {
                    (*threshold_db + (env_db - *threshold_db) / r_val) - env_db
                } else {
                    0.0
                };
                let g = db_to_linear(gr_db);
                // Attenuate sibilant band cleanly while leaving low/mid body intact.
                s[0] = (s[0] - sib_l) + sib_l * g;
                s[1] = (s[1] - sib_r) + sib_r * g;
            }
        }
        AudioFilter::AudioDelay {
            delay_ms,
            feedback,
            wet_mix,
        } => {
            let delay_samples = ((delay_ms.clamp(1.0, 2000.0) as f64 / 1000.0) * sr) as usize;
            rt.delay_line.set_delay_samples(delay_samples);
            let fb = feedback.clamp(0.0, 0.95);
            let wet = wet_mix.clamp(0.0, 1.0);
            for s in buf {
                let dry_l = s[0];
                let dry_r = s[1];
                let delayed = rt.delay_line.process_with_feedback(dry_l, dry_r, fb);
                s[0] = dry_l * (1.0 - wet) + delayed[0] * wet;
                s[1] = dry_r * (1.0 - wet) + delayed[1] * wet;
            }
        }
    }
}

/// Pure-Rust in-place radix-2 Cooley-Tukey FFT (`no_std`).
fn fft_radix2(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    if n <= 1 || !n.is_power_of_two() {
        return;
    }

    // Bit-reversal permutation.
    let mut j = 0_usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while (j & bit) != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let sign = if inverse { 1.0_f32 } else { -1.0_f32 };
    let mut len = 2_usize;
    while len <= n {
        let ang = sign * 2.0 * core::f32::consts::PI / len as f32;
        let wlen_r = cosf(ang);
        let wlen_i = sinf(ang);
        let half = len >> 1;

        for start in (0..n).step_by(len) {
            let mut wr = 1.0_f32;
            let mut wi = 0.0_f32;
            for k in 0..half {
                let i0 = start + k;
                let i1 = i0 + half;
                let tr = re[i1] * wr - im[i1] * wi;
                let ti = re[i1] * wi + im[i1] * wr;
                re[i1] = re[i0] - tr;
                im[i1] = im[i0] - ti;
                re[i0] += tr;
                im[i0] += ti;

                let next_wr = wr * wlen_r - wi * wlen_i;
                wi = wr * wlen_i + wi * wlen_r;
                wr = next_wr;
            }
        }
        len <<= 1;
    }

    if inverse {
        let inv_n = 1.0 / n as f32;
        for i in 0..n {
            re[i] *= inv_n;
            im[i] *= inv_n;
        }
    }
}

fn apply_spectral_suppress(
    buf: &mut [[f32; 2]],
    rt: &mut AudioFilterRuntime,
    threshold_db: f32,
    reduction_db: f32,
) {
    const N: usize = 64;
    if buf.is_empty() {
        return;
    }
    let thresh_mag = db_to_linear(threshold_db) * (N as f32 * 0.5);
    let floor_gain = db_to_linear(-absf(reduction_db));

    for chunk in buf.chunks_mut(N) {
        let len = chunk.len();
        let mut re_l = [0.0_f32; N];
        let mut im_l = [0.0_f32; N];
        let mut re_r = [0.0_f32; N];
        let mut im_r = [0.0_f32; N];
        for (i, s) in chunk.iter().enumerate() {
            re_l[i] = s[0];
            re_r[i] = s[1];
        }
        fft_radix2(&mut re_l, &mut im_l, false);
        fft_radix2(&mut re_r, &mut im_r, false);

        for k in 0..N {
            let mag_l = sqrtf(re_l[k] * re_l[k] + im_l[k] * im_l[k]);
            let gain_l = if mag_l < thresh_mag { floor_gain } else { 1.0 };
            re_l[k] *= gain_l;
            im_l[k] *= gain_l;

            let mag_r = sqrtf(re_r[k] * re_r[k] + im_r[k] * im_r[k]);
            let gain_r = if mag_r < thresh_mag { floor_gain } else { 1.0 };
            re_r[k] *= gain_r;
            im_r[k] *= gain_r;
        }

        fft_radix2(&mut re_l, &mut im_l, true);
        fft_radix2(&mut re_r, &mut im_r, true);

        for i in 0..len {
            chunk[i][0] = re_l[i];
            chunk[i][1] = re_r[i];
        }
    }
    let _ = (&rt.fft_hist_l, &rt.fft_hist_r);
}

/// Computes static powf helper cleanly without unused warning.
#[allow(dead_code)]
fn _keep_powf(a: f32, b: f32) -> f32 {
    powf(a, b)
}
