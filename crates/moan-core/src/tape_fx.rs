// SPDX-License-Identifier: GPL-3.0-only
//! Sidechain dynamics (compressor, gate with hysteresis/hold) and studio insert FX
//! (tape machine, transient shaper, bit crusher, quadrature stereo chorus).
//! All dynamic buffers allocate strictly at construction via `try_reserve_exact`.

use crate::{
    dsp_math::{abs_f32, cos_f32, exp_f32, floor_f32, log10_f32, powf_f32, round_f32, sin_f32, TAU},
    parametric_eq::{f32_to_frame, frame_to_f32, BandKind, Biquad},
    Frame, RATE,
};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Threshold,
    Ratio,
    Timing,
    Gain,
    Range,
    Rate,
    Depth,
    Feedback,
    Mix,
    Memory,
}

#[inline]
fn time_ms_coeff(ms: f32) -> f32 {
    exp_f32(-1.0 / ((ms * 0.001 * RATE as f32).max(1.0)))
}

#[inline]
fn one_pole_alpha(hz: f32) -> f32 {
    if hz <= 0.0 {
        return 0.0;
    }
    1.0 - exp_f32(-TAU * hz / RATE as f32)
}

#[inline]
fn read_delay_cubic(buf: &[[f32; 2]], write_idx: usize, delay_frames: f32) -> [f32; 2] {
    let n = buf.len();
    let max_delay = (n.saturating_sub(3)) as f32;
    let d = delay_frames.clamp(1.0, max_delay.max(1.0));
    let read_pos = (write_idx as f32) + (n as f32) - d;
    let base = floor_f32(read_pos);
    let frac = read_pos - base;
    let i0 = (base as usize) % n;
    let im1 = (i0 + n - 1) % n;
    let i1 = (i0 + 1) % n;
    let i2 = (i0 + 2) % n;

    let mut out = [0.0f32; 2];
    for c in 0..2 {
        let xm1 = buf[im1][c];
        let x0 = buf[i0][c];
        let x1 = buf[i1][c];
        let x2 = buf[i2][c];
        let c0 = x0;
        let c1 = 0.5 * (x1 - xm1);
        let c2 = xm1 - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
        let c3 = 0.5 * (x2 - xm1) + 1.5 * (x0 - x1);
        out[c] = ((c3 * frac + c2) * frac + c1) * frac + c0;
    }
    out
}

/// Soft-knee stereo-linked compressor with 80 Hz 1-pole sidechain high-pass,
/// optional external sidechain key, and live gain reduction metering (`>= 0.0 dB`).
#[derive(Clone, Debug)]
pub struct SidechainCompressor {
    threshold_db: f32,
    ratio: f32,
    attack_ms: f32,
    release_ms: f32,
    makeup_db: f32,
    knee_db: f32,
    sc_hpf_hz: f32,
    attack_coeff: f32,
    release_coeff: f32,
    sc_hpf_alpha: f32,
    makeup_lin: f32,
    sc_lp_state: [f32; 2],
    gr_db: f32,
}

impl SidechainCompressor {
    pub fn new(
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    ) -> Result<Self, Error> {
        if !threshold_db.is_finite() || !(-60.0..=0.0).contains(&threshold_db) {
            return Err(Error::Threshold);
        }
        if !ratio.is_finite() || !(1.0..=20.0).contains(&ratio) {
            return Err(Error::Ratio);
        }
        if !attack_ms.is_finite() || !(0.1..=500.0).contains(&attack_ms) {
            return Err(Error::Timing);
        }
        if !release_ms.is_finite() || !(5.0..=2000.0).contains(&release_ms) {
            return Err(Error::Timing);
        }
        if !makeup_db.is_finite() || !(-12.0..=24.0).contains(&makeup_db) {
            return Err(Error::Gain);
        }

        let sc_hpf_hz = 80.0;
        Ok(Self {
            threshold_db,
            ratio,
            attack_ms,
            release_ms,
            makeup_db,
            knee_db: 6.0,
            sc_hpf_hz,
            attack_coeff: time_ms_coeff(attack_ms),
            release_coeff: time_ms_coeff(release_ms),
            sc_hpf_alpha: one_pole_alpha(sc_hpf_hz),
            makeup_lin: powf_f32(10.0, makeup_db / 20.0),
            sc_lp_state: [0.0; 2],
            gr_db: 0.0,
        })
    }

    pub fn set_knee_db(&mut self, knee_db: f32) -> Result<(), Error> {
        if !knee_db.is_finite() || !(0.0..=24.0).contains(&knee_db) {
            return Err(Error::Range);
        }
        self.knee_db = knee_db;
        Ok(())
    }

    pub fn set_sidechain_hpf_hz(&mut self, hz: f32) -> Result<(), Error> {
        if !hz.is_finite() || !(0.0..=500.0).contains(&hz) {
            return Err(Error::Range);
        }
        self.sc_hpf_hz = hz;
        self.sc_hpf_alpha = one_pole_alpha(hz);
        Ok(())
    }

    pub fn set_parameters(
        &mut self,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    ) -> Result<(), Error> {
        let updated = Self::new(threshold_db, ratio, attack_ms, release_ms, makeup_db)?;
        self.threshold_db = updated.threshold_db;
        self.ratio = updated.ratio;
        self.attack_ms = updated.attack_ms;
        self.release_ms = updated.release_ms;
        self.makeup_db = updated.makeup_db;
        self.attack_coeff = updated.attack_coeff;
        self.release_coeff = updated.release_coeff;
        self.makeup_lin = updated.makeup_lin;
        Ok(())
    }

    pub fn parameters(&self) -> (f32, f32, f32, f32, f32) {
        (
            self.threshold_db,
            self.ratio,
            self.attack_ms,
            self.release_ms,
            self.makeup_db,
        )
    }

    /// Live positive gain reduction in dB (`0.0` when idle, `> 0.0` when compressing).
    #[inline]
    pub fn gain_reduction_db(&self) -> f32 {
        self.gr_db
    }

    pub fn reset(&mut self) {
        self.sc_lp_state = [0.0; 2];
        self.gr_db = 0.0;
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2], key: Option<[f32; 2]>) -> [f32; 2] {
        let det_in = key.unwrap_or(input);
        let det_peak = if self.sc_hpf_hz > 0.0 {
            let mut hp = [0.0f32; 2];
            for c in 0..2 {
                self.sc_lp_state[c] += self.sc_hpf_alpha * (det_in[c] - self.sc_lp_state[c]);
                hp[c] = det_in[c] - self.sc_lp_state[c];
            }
            abs_f32(hp[0]).max(abs_f32(hp[1]))
        } else {
            abs_f32(det_in[0]).max(abs_f32(det_in[1]))
        };

        let det_db = if det_peak > 1e-6 {
            20.0 * log10_f32(det_peak)
        } else {
            -120.0
        };

        let slope = 1.0 - 1.0 / self.ratio;
        let over = det_db - self.threshold_db;
        let half_knee = 0.5 * self.knee_db;
        let target_gr_db = if self.knee_db > 1e-4 && abs_f32(over) <= half_knee {
            let t = over + half_knee;
            slope * (t * t) / (2.0 * self.knee_db)
        } else if over > half_knee {
            slope * over
        } else {
            0.0
        };

        let coeff = if target_gr_db > self.gr_db {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.gr_db = (target_gr_db + coeff * (self.gr_db - target_gr_db)).max(0.0);

        let gain = powf_f32(10.0, -self.gr_db / 20.0) * self.makeup_lin;
        [input[0] * gain, input[1] * gain]
    }

    #[inline]
    pub fn process_with_key(&mut self, input: Frame, sidechain_key: Option<Frame>) -> Frame {
        let out = self.process_stereo_f32(frame_to_f32(input), sidechain_key.map(frame_to_f32));
        f32_to_frame(out)
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self.process_with_key(*f, None);
        }
    }
}

/// Noise / ducking gate with threshold, hysteresis, hold time, attack/release,
/// attenuation range (`-80..-3 dB`), 80 Hz sidechain HPF, and external key input.
#[derive(Clone, Debug)]
pub struct SidechainGate {
    threshold_db: f32,
    hysteresis_db: f32,
    hold_ms: f32,
    attack_ms: f32,
    release_ms: f32,
    range_db: f32,
    hold_frames: u32,
    hold_counter: u32,
    attack_coeff: f32,
    release_coeff: f32,
    sc_hpf_hz: f32,
    sc_hpf_alpha: f32,
    sc_lp_state: [f32; 2],
    is_open: bool,
    attenuation_db: f32,
}

impl SidechainGate {
    pub fn new(
        threshold_db: f32,
        hysteresis_db: f32,
        hold_ms: f32,
        attack_ms: f32,
        release_ms: f32,
        range_db: f32,
    ) -> Result<Self, Error> {
        if !threshold_db.is_finite() || !(-80.0..=0.0).contains(&threshold_db) {
            return Err(Error::Threshold);
        }
        if !hysteresis_db.is_finite() || !(0.0..=24.0).contains(&hysteresis_db) {
            return Err(Error::Range);
        }
        if !hold_ms.is_finite() || !(0.0..=2000.0).contains(&hold_ms) {
            return Err(Error::Timing);
        }
        if !attack_ms.is_finite() || !(0.1..=500.0).contains(&attack_ms) {
            return Err(Error::Timing);
        }
        if !release_ms.is_finite() || !(1.0..=5000.0).contains(&release_ms) {
            return Err(Error::Timing);
        }
        if !range_db.is_finite() || !(-80.0..=-3.0).contains(&range_db) {
            return Err(Error::Range);
        }

        let sc_hpf_hz = 80.0;
        let hold_frames = round_f32(hold_ms * 0.001 * RATE as f32) as u32;
        Ok(Self {
            threshold_db,
            hysteresis_db,
            hold_ms,
            attack_ms,
            release_ms,
            range_db,
            hold_frames,
            hold_counter: 0,
            attack_coeff: time_ms_coeff(attack_ms),
            release_coeff: time_ms_coeff(release_ms),
            sc_hpf_hz,
            sc_hpf_alpha: one_pole_alpha(sc_hpf_hz),
            sc_lp_state: [0.0; 2],
            is_open: false,
            attenuation_db: -range_db,
        })
    }

    pub fn set_sidechain_hpf_hz(&mut self, hz: f32) -> Result<(), Error> {
        if !hz.is_finite() || !(0.0..=500.0).contains(&hz) {
            return Err(Error::Range);
        }
        self.sc_hpf_hz = hz;
        self.sc_hpf_alpha = one_pole_alpha(hz);
        Ok(())
    }

    pub fn parameters(&self) -> (f32, f32, f32, f32, f32, f32) {
        (
            self.threshold_db,
            self.hysteresis_db,
            self.hold_ms,
            self.attack_ms,
            self.release_ms,
            self.range_db,
        )
    }

    #[inline]
    pub fn is_open(&self) -> bool {
        self.is_open
    }

    /// Live positive gain reduction in dB (`0.0` when open, up to `-range_db` when closed).
    #[inline]
    pub fn gain_reduction_db(&self) -> f32 {
        self.attenuation_db
    }

    pub fn reset(&mut self) {
        self.sc_lp_state = [0.0; 2];
        self.hold_counter = 0;
        self.is_open = false;
        self.attenuation_db = -self.range_db;
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2], key: Option<[f32; 2]>) -> [f32; 2] {
        let det_in = key.unwrap_or(input);
        let det_peak = if self.sc_hpf_hz > 0.0 {
            let mut hp = [0.0f32; 2];
            for c in 0..2 {
                self.sc_lp_state[c] += self.sc_hpf_alpha * (det_in[c] - self.sc_lp_state[c]);
                hp[c] = det_in[c] - self.sc_lp_state[c];
            }
            abs_f32(hp[0]).max(abs_f32(hp[1]))
        } else {
            abs_f32(det_in[0]).max(abs_f32(det_in[1]))
        };

        let det_db = if det_peak > 1e-6 {
            20.0 * log10_f32(det_peak)
        } else {
            -120.0
        };

        let close_thresh = self.threshold_db - self.hysteresis_db;
        if !self.is_open {
            if det_db >= self.threshold_db {
                self.is_open = true;
                self.hold_counter = self.hold_frames;
            }
        } else if det_db >= close_thresh {
            self.hold_counter = self.hold_frames;
        } else if self.hold_counter > 0 {
            self.hold_counter -= 1;
        } else {
            self.is_open = false;
        }

        let target_att_db = if self.is_open { 0.0 } else { -self.range_db };
        let coeff = if target_att_db < self.attenuation_db {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        self.attenuation_db =
            (target_att_db + coeff * (self.attenuation_db - target_att_db)).clamp(0.0, -self.range_db);

        let gain = if self.range_db <= -79.5 && self.attenuation_db >= (-self.range_db - 0.5) {
            0.0
        } else {
            powf_f32(10.0, -self.attenuation_db / 20.0)
        };

        [input[0] * gain, input[1] * gain]
    }

    #[inline]
    pub fn process_with_key(&mut self, input: Frame, sidechain_key: Option<Frame>) -> Frame {
        let out = self.process_stereo_f32(frame_to_f32(input), sidechain_key.map(frame_to_f32));
        f32_to_frame(out)
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self.process_with_key(*f, None);
        }
    }
}

/// Analog tape emulation with pre-allocated 6 ms stereo modulated delay line,
/// asymmetric soft saturation, 85 Hz head bump, tone high-cut, dual wow/flutter LFOs,
/// signal-envelope-gated hiss, and latency-aligned dry/wet mix.
#[derive(Clone, Debug)]
pub struct TapeMachine {
    drive: f32,
    head_bump_db: f32,
    tone_hz: f32,
    wow: f32,
    flutter: f32,
    hiss: f32,
    mix: f32,
    wet_buf: Vec<[f32; 2]>,
    dry_buf: Vec<[f32; 2]>,
    wet_pos: usize,
    dry_pos: usize,
    head_bump: Biquad,
    tone_lp: [f32; 2],
    tone_alpha: f32,
    dc_lp: [f32; 2],
    dc_alpha: f32,
    wow_phase: f32,
    flutter_phase: f32,
    sig_env: f32,
    hiss_gate: f32,
    hiss_lp: [f32; 2],
    rng_state: u32,
}

impl TapeMachine {
    /// 6 ms circular buffer (288 frames at 48 kHz) centered at 144 frames (3.0 ms).
    pub const DELAY_CAPACITY_FRAMES: usize = 288;
    pub const LATENCY_FRAMES: usize = 144;

    pub fn new(
        drive: f32,
        head_bump_db: f32,
        tone_hz: f32,
        wow: f32,
        flutter: f32,
        hiss: f32,
        mix: f32,
    ) -> Result<Self, Error> {
        if !drive.is_finite() || !(0.0..=1.0).contains(&drive) {
            return Err(Error::Gain);
        }
        if !head_bump_db.is_finite() || !(0.0..=5.0).contains(&head_bump_db) {
            return Err(Error::Gain);
        }
        if !tone_hz.is_finite() || !(1500.0..=20000.0).contains(&tone_hz) {
            return Err(Error::Range);
        }
        if !wow.is_finite() || !(0.0..=1.0).contains(&wow) {
            return Err(Error::Depth);
        }
        if !flutter.is_finite() || !(0.0..=1.0).contains(&flutter) {
            return Err(Error::Depth);
        }
        if !hiss.is_finite() || !(0.0..=1.0).contains(&hiss) {
            return Err(Error::Gain);
        }
        if !mix.is_finite() || !(0.0..=1.0).contains(&mix) {
            return Err(Error::Mix);
        }

        let mut wet_buf = Vec::new();
        wet_buf
            .try_reserve_exact(Self::DELAY_CAPACITY_FRAMES)
            .map_err(|_| Error::Memory)?;
        wet_buf.resize(Self::DELAY_CAPACITY_FRAMES, [0.0; 2]);

        let mut dry_buf = Vec::new();
        dry_buf
            .try_reserve_exact(Self::LATENCY_FRAMES)
            .map_err(|_| Error::Memory)?;
        dry_buf.resize(Self::LATENCY_FRAMES, [0.0; 2]);

        let mut head_bump = Biquad::default();
        head_bump.configure(BandKind::Bell, 85.0, head_bump_db, 1.6);

        Ok(Self {
            drive,
            head_bump_db,
            tone_hz,
            wow,
            flutter,
            hiss,
            mix,
            wet_buf,
            dry_buf,
            wet_pos: 0,
            dry_pos: 0,
            head_bump,
            tone_lp: [0.0; 2],
            tone_alpha: one_pole_alpha(tone_hz),
            dc_lp: [0.0; 2],
            dc_alpha: one_pole_alpha(10.0),
            wow_phase: 0.0,
            flutter_phase: 0.0,
            sig_env: 0.0,
            hiss_gate: 0.0,
            hiss_lp: [0.0; 2],
            rng_state: 0x5A17_C0DE,
        })
    }

    pub fn latency_frames(&self) -> usize {
        Self::LATENCY_FRAMES
    }

    pub fn parameters(&self) -> (f32, f32, f32, f32, f32, f32, f32) {
        (
            self.drive,
            self.head_bump_db,
            self.tone_hz,
            self.wow,
            self.flutter,
            self.hiss,
            self.mix,
        )
    }

    pub fn reset(&mut self) {
        self.wet_buf.fill([0.0; 2]);
        self.dry_buf.fill([0.0; 2]);
        self.wet_pos = 0;
        self.dry_pos = 0;
        self.head_bump.reset();
        self.tone_lp = [0.0; 2];
        self.dc_lp = [0.0; 2];
        self.wow_phase = 0.0;
        self.flutter_phase = 0.0;
        self.sig_env = 0.0;
        self.hiss_gate = 0.0;
        self.hiss_lp = [0.0; 2];
        self.rng_state = 0x5A17_C0DE;
    }

    #[inline]
    fn next_noise(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        // Map u32 to [-1.0, 1.0]
        ((x >> 8) as f32 / 8388608.0) - 1.0
    }

    #[inline]
    fn saturate_asym(x: f32, gain: f32, bias: f32) -> f32 {
        #[inline]
        fn soft(v: f32) -> f32 {
            v / (1.0 + abs_f32(v))
        }
        let d = x * gain + bias;
        (soft(d) - soft(bias)) * 1.15
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2]) -> [f32; 2] {
        // Latency-aligned dry delay
        let dry_aligned = self.dry_buf[self.dry_pos];
        self.dry_buf[self.dry_pos] = input;
        self.dry_pos = (self.dry_pos + 1) % Self::LATENCY_FRAMES;

        // Signal-envelope-gated hiss (strictly 0.0 when input is silent)
        let peak = abs_f32(input[0]).max(abs_f32(input[1]));
        let att_c = time_ms_coeff(2.0);
        let rel_c = time_ms_coeff(40.0);
        let env_c = if peak > self.sig_env { att_c } else { rel_c };
        self.sig_env = peak + env_c * (self.sig_env - peak);
        if peak == 0.0 && self.sig_env < 1e-5 {
            self.sig_env = 0.0;
        }

        let target_gate = if self.sig_env > 1e-4 { 1.0 } else { 0.0 };
        let gate_c = if target_gate > self.hiss_gate {
            time_ms_coeff(5.0)
        } else {
            time_ms_coeff(35.0)
        };
        self.hiss_gate = target_gate + gate_c * (self.hiss_gate - target_gate);
        if self.sig_env == 0.0 && self.hiss_gate < 1e-4 {
            self.hiss_gate = 0.0;
        }

        // Asymmetric soft saturation + DC blocker
        let sat_gain = 1.0 + self.drive * 4.5;
        let sat_bias = 0.08 * self.drive;
        let mut wet = [0.0f32; 2];
        for c in 0..2 {
            let s = if self.drive > 0.0 {
                Self::saturate_asym(input[c], sat_gain, sat_bias)
            } else {
                input[c]
            };
            self.dc_lp[c] += self.dc_alpha * (s - self.dc_lp[c]);
            wet[c] = s - self.dc_lp[c];
        }

        // 85 Hz head bump
        wet = self.head_bump.process(wet);

        // Add signal-gated tape hiss before high-cut tone filter
        if self.hiss > 0.0 && self.hiss_gate > 0.0 {
            let hiss_amp = self.hiss * 0.012 * self.hiss_gate;
            for c in 0..2 {
                let raw_n = self.next_noise();
                self.hiss_lp[c] += 0.35 * (raw_n - self.hiss_lp[c]);
                wet[c] += self.hiss_lp[c] * hiss_amp;
            }
        }

        // Gentle high-cut tone rolloff
        for c in 0..2 {
            self.tone_lp[c] += self.tone_alpha * (wet[c] - self.tone_lp[c]);
            wet[c] = self.tone_lp[c];
        }

        // Dual LFO wow (~0.55 Hz) & flutter (~6.8 Hz) fractional delay
        self.wet_buf[self.wet_pos] = wet;
        let mod_frames = self.wow * 96.0 * sin_f32(TAU * self.wow_phase)
            + self.flutter * 12.0 * sin_f32(TAU * self.flutter_phase);
        let delay_frames = (Self::LATENCY_FRAMES as f32 + mod_frames)
            .clamp(1.0, (Self::DELAY_CAPACITY_FRAMES - 3) as f32);
        let wet_delayed = read_delay_cubic(&self.wet_buf, self.wet_pos, delay_frames);

        self.wet_pos = (self.wet_pos + 1) % Self::DELAY_CAPACITY_FRAMES;
        self.wow_phase += 0.55 / RATE as f32;
        if self.wow_phase >= 1.0 {
            self.wow_phase -= 1.0;
        }
        self.flutter_phase += 6.8 / RATE as f32;
        if self.flutter_phase >= 1.0 {
            self.flutter_phase -= 1.0;
        }

        let dry_mix = 1.0 - self.mix;
        [
            dry_aligned[0] * dry_mix + wet_delayed[0] * self.mix,
            dry_aligned[1] * dry_mix + wet_delayed[1] * self.mix,
        ]
    }

    #[inline]
    pub fn process_frame(&mut self, frame: Frame) -> Frame {
        f32_to_frame(self.process_stereo_f32(frame_to_f32(frame)))
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self.process_frame(*f);
        }
    }
}

/// Dual fast/slow envelope differential attack (`-100..+100%`) and sustain (`-100..+100%`) shaper.
#[derive(Clone, Debug)]
pub struct TransientShaper {
    attack_pct: f32,
    sustain_pct: f32,
    env_fast_att: f32,
    env_slow_att: f32,
    env_fast_sus: f32,
    env_slow_sus: f32,
    c_fast_att: f32,
    c_slow_att: f32,
    c_att_rel: f32,
    c_sus_att: f32,
    c_fast_sus_rel: f32,
    c_slow_sus_rel: f32,
}

impl TransientShaper {
    pub fn new(attack_pct: f32, sustain_pct: f32) -> Result<Self, Error> {
        if !attack_pct.is_finite() || !(-100.0..=100.0).contains(&attack_pct) {
            return Err(Error::Gain);
        }
        if !sustain_pct.is_finite() || !(-100.0..=100.0).contains(&sustain_pct) {
            return Err(Error::Gain);
        }
        Ok(Self {
            attack_pct,
            sustain_pct,
            env_fast_att: 0.0,
            env_slow_att: 0.0,
            env_fast_sus: 0.0,
            env_slow_sus: 0.0,
            c_fast_att: time_ms_coeff(1.0),
            c_slow_att: time_ms_coeff(18.0),
            c_att_rel: time_ms_coeff(25.0),
            c_sus_att: time_ms_coeff(5.0),
            c_fast_sus_rel: time_ms_coeff(40.0),
            c_slow_sus_rel: time_ms_coeff(260.0),
        })
    }

    pub fn parameters(&self) -> (f32, f32) {
        (self.attack_pct, self.sustain_pct)
    }

    pub fn reset(&mut self) {
        self.env_fast_att = 0.0;
        self.env_slow_att = 0.0;
        self.env_fast_sus = 0.0;
        self.env_slow_sus = 0.0;
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2]) -> [f32; 2] {
        let det = abs_f32(input[0]).max(abs_f32(input[1]));

        let cfa = if det > self.env_fast_att {
            self.c_fast_att
        } else {
            self.c_att_rel
        };
        let csa = if det > self.env_slow_att {
            self.c_slow_att
        } else {
            self.c_att_rel
        };
        self.env_fast_att = det + cfa * (self.env_fast_att - det);
        self.env_slow_att = det + csa * (self.env_slow_att - det);

        let cfs = if det > self.env_fast_sus {
            self.c_sus_att
        } else {
            self.c_fast_sus_rel
        };
        let css = if det > self.env_slow_sus {
            self.c_sus_att
        } else {
            self.c_slow_sus_rel
        };
        self.env_fast_sus = det + cfs * (self.env_fast_sus - det);
        self.env_slow_sus = det + css * (self.env_slow_sus - det);

        if abs_f32(self.attack_pct) < 1e-4 && abs_f32(self.sustain_pct) < 1e-4 {
            return input;
        }

        let att_diff = if self.env_fast_att > 1e-5 {
            ((self.env_fast_att - self.env_slow_att) / self.env_fast_att).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sus_diff = if self.env_slow_sus > 1e-5 {
            ((self.env_slow_sus - self.env_fast_sus) / self.env_slow_sus).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let att_gain = (1.0 + (self.attack_pct / 100.0) * att_diff * 2.0).clamp(0.1, 4.0);
        let sus_gain = (1.0 + (self.sustain_pct / 100.0) * sus_diff * 2.0).clamp(0.05, 4.0);
        let gain = (att_gain * sus_gain).clamp(0.05, 4.0);
        [input[0] * gain, input[1] * gain]
    }

    #[inline]
    pub fn process_frame(&mut self, frame: Frame) -> Frame {
        f32_to_frame(self.process_stereo_f32(frame_to_f32(frame)))
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self.process_frame(*f);
        }
    }
}

/// Sample-and-hold downsampler (`1..=64`) and bit-depth quantizer (`2..=16` bits) with dry/wet mix.
#[derive(Clone, Debug)]
pub struct BitCrusher {
    downsample: u16,
    bits: u8,
    mix: f32,
    hold_counter: u16,
    held: [f32; 2],
}

impl BitCrusher {
    pub fn new(downsample: u16, bits: u8, mix: f32) -> Result<Self, Error> {
        if !(1..=64).contains(&downsample) {
            return Err(Error::Range);
        }
        if !(2..=16).contains(&bits) {
            return Err(Error::Range);
        }
        if !mix.is_finite() || !(0.0..=1.0).contains(&mix) {
            return Err(Error::Mix);
        }
        Ok(Self {
            downsample,
            bits,
            mix,
            hold_counter: 0,
            held: [0.0; 2],
        })
    }

    pub fn parameters(&self) -> (u16, u8, f32) {
        (self.downsample, self.bits, self.mix)
    }

    pub fn reset(&mut self) {
        self.hold_counter = 0;
        self.held = [0.0; 2];
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2]) -> [f32; 2] {
        if self.hold_counter == 0 {
            let scale = (1u32 << (self.bits - 1)) as f32;
            self.held = [
                (round_f32(input[0] * scale) / scale).clamp(-1.0, 1.0),
                (round_f32(input[1] * scale) / scale).clamp(-1.0, 1.0),
            ];
        }
        self.hold_counter += 1;
        if self.hold_counter >= self.downsample {
            self.hold_counter = 0;
        }

        let dry = 1.0 - self.mix;
        [
            input[0] * dry + self.held[0] * self.mix,
            input[1] * dry + self.held[1] * self.mix,
        ]
    }

    #[inline]
    pub fn process_frame(&mut self, frame: Frame) -> Frame {
        f32_to_frame(self.process_stereo_f32(frame_to_f32(frame)))
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self.process_frame(*f);
        }
    }
}

/// Quadrature stereo LFO modulated delay chorus.
#[derive(Clone, Debug)]
pub struct StereoChorus {
    rate_hz: f32,
    depth_ms: f32,
    feedback: f32,
    mix: f32,
    delay_buf: Vec<[f32; 2]>,
    write_pos: usize,
    phase: f32,
}

impl StereoChorus {
    const BASE_DELAY_FRAMES: f32 = 740.0; // ~15.4 ms at 48 kHz so max 15 ms depth stays >= 20 frames
    const BUFFER_CAPACITY: usize = 1600; // > 15.4 ms + 15 ms depth + cubic margin

    pub fn new(rate_hz: f32, depth_ms: f32, feedback: f32, mix: f32) -> Result<Self, Error> {
        if !rate_hz.is_finite() || !(0.05..=10.0).contains(&rate_hz) {
            return Err(Error::Rate);
        }
        if !depth_ms.is_finite() || !(0.0..=15.0).contains(&depth_ms) {
            return Err(Error::Depth);
        }
        if !feedback.is_finite() || !(-0.9..=0.9).contains(&feedback) {
            return Err(Error::Feedback);
        }
        if !mix.is_finite() || !(0.0..=1.0).contains(&mix) {
            return Err(Error::Mix);
        }

        let mut delay_buf = Vec::new();
        delay_buf
            .try_reserve_exact(Self::BUFFER_CAPACITY)
            .map_err(|_| Error::Memory)?;
        delay_buf.resize(Self::BUFFER_CAPACITY, [0.0; 2]);

        Ok(Self {
            rate_hz,
            depth_ms,
            feedback,
            mix,
            delay_buf,
            write_pos: 0,
            phase: 0.0,
        })
    }

    pub fn parameters(&self) -> (f32, f32, f32, f32) {
        (self.rate_hz, self.depth_ms, self.feedback, self.mix)
    }

    pub fn reset(&mut self) {
        self.delay_buf.fill([0.0; 2]);
        self.write_pos = 0;
        self.phase = 0.0;
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2]) -> [f32; 2] {
        let angle = TAU * self.phase;
        let depth_frames = self.depth_ms * 0.001 * RATE as f32;
        let delay_l = Self::BASE_DELAY_FRAMES + depth_frames * sin_f32(angle);
        let delay_r = Self::BASE_DELAY_FRAMES + depth_frames * cos_f32(angle);

        let wet_l = read_delay_cubic(&self.delay_buf, self.write_pos, delay_l)[0];
        let wet_r = read_delay_cubic(&self.delay_buf, self.write_pos, delay_r)[1];

        self.delay_buf[self.write_pos] = [
            (input[0] + wet_l * self.feedback).clamp(-4.0, 4.0),
            (input[1] + wet_r * self.feedback).clamp(-4.0, 4.0),
        ];
        self.write_pos = (self.write_pos + 1) % Self::BUFFER_CAPACITY;

        self.phase += self.rate_hz / RATE as f32;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }

        let dry = 1.0 - self.mix;
        [
            input[0] * dry + wet_l * self.mix,
            input[1] * dry + wet_r * self.mix,
        ]
    }

    #[inline]
    pub fn process_frame(&mut self, frame: Frame) -> Frame {
        f32_to_frame(self.process_stereo_f32(frame_to_f32(frame)))
    }

    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self.process_frame(*f);
        }
    }
}
