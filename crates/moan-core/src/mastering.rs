// SPDX-License-Identifier: GPL-3.0-only
//! Master finishing chain and 4 ms (192-frame at 48 kHz) true-peak lookahead brickwall limiter.
//! Allocates strictly at construction via `try_reserve_exact`; zero allocations during rendering.

use crate::{
    dsp_math::{abs_f32, exp_f32, log10_f32, powf_f32},
    parametric_eq::{f32_to_frame, frame_to_f32, BandKind, Biquad},
    tape_fx::SidechainCompressor,
    Frame, RATE,
};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Ceiling,
    Timing,
    Gain,
    Width,
    Dynamics,
    Memory,
}

/// 4.0 ms (192-frame at 48 kHz) stereo lookahead brickwall limiter with
/// 2x midpoint inter-sample peak estimation, smooth 192-frame attack ramp,
/// configurable release, and live gain reduction metering (`>= 0.0 dB`).
#[derive(Clone, Debug)]
pub struct LookaheadLimiter {
    ceiling_db: f32,
    ceiling_lin: f32,
    release_ms: f32,
    release_coeff: f32,
    delay_buf: Vec<[f32; 2]>,
    env_buf: Vec<f32>,
    pos: usize,
    hist: [[f32; 2]; 3],
    gain_env: f32,
    current_gr_db: f32,
}

impl LookaheadLimiter {
    pub const LOOKAHEAD_FRAMES: usize = 192; // 4.0 ms at 48 kHz

    pub fn new(ceiling_db: f32, release_ms: f32) -> Result<Self, Error> {
        if !ceiling_db.is_finite() || !(-24.0..=0.0).contains(&ceiling_db) {
            return Err(Error::Ceiling);
        }
        if !release_ms.is_finite() || !(10.0..=1000.0).contains(&release_ms) {
            return Err(Error::Timing);
        }

        let mut delay_buf = Vec::new();
        delay_buf
            .try_reserve_exact(Self::LOOKAHEAD_FRAMES)
            .map_err(|_| Error::Memory)?;
        delay_buf.resize(Self::LOOKAHEAD_FRAMES, [0.0; 2]);

        let mut env_buf = Vec::new();
        env_buf
            .try_reserve_exact(Self::LOOKAHEAD_FRAMES)
            .map_err(|_| Error::Memory)?;
        env_buf.resize(Self::LOOKAHEAD_FRAMES, 1.0);

        Ok(Self {
            ceiling_db,
            ceiling_lin: powf_f32(10.0, ceiling_db / 20.0) * 0.9999,
            release_ms,
            release_coeff: exp_f32(-1.0 / (release_ms * 0.001 * RATE as f32)),
            delay_buf,
            env_buf,
            pos: 0,
            hist: [[0.0; 2]; 3],
            gain_env: 1.0,
            current_gr_db: 0.0,
        })
    }

    pub fn default_master() -> Result<Self, Error> {
        Self::new(-1.0, 100.0)
    }

    pub fn set_ceiling_db(&mut self, ceiling_db: f32) -> Result<(), Error> {
        if !ceiling_db.is_finite() || !(-24.0..=0.0).contains(&ceiling_db) {
            return Err(Error::Ceiling);
        }
        self.ceiling_db = ceiling_db;
        self.ceiling_lin = powf_f32(10.0, ceiling_db / 20.0) * 0.9999;
        Ok(())
    }

    pub fn set_release_ms(&mut self, release_ms: f32) -> Result<(), Error> {
        if !release_ms.is_finite() || !(10.0..=1000.0).contains(&release_ms) {
            return Err(Error::Timing);
        }
        self.release_ms = release_ms;
        self.release_coeff = exp_f32(-1.0 / (release_ms * 0.001 * RATE as f32));
        Ok(())
    }

    pub fn ceiling_db(&self) -> f32 {
        self.ceiling_db
    }

    pub fn release_ms(&self) -> f32 {
        self.release_ms
    }

    pub fn latency_frames(&self) -> usize {
        Self::LOOKAHEAD_FRAMES
    }

    /// Live positive gain reduction in dB (`0.0` when idle, `> 0.0` when limiting).
    #[inline]
    pub fn gain_reduction_db(&self) -> f32 {
        self.current_gr_db
    }

    pub fn reset(&mut self) {
        self.delay_buf.fill([0.0; 2]);
        self.env_buf.fill(1.0);
        self.pos = 0;
        self.hist = [[0.0; 2]; 3];
        self.gain_env = 1.0;
        self.current_gr_db = 0.0;
    }

    #[inline]
    fn schedule_ramp(&mut self, anchor_pos: usize, max_span: usize, req_gain: f32) {
        let n = Self::LOOKAHEAD_FRAMES;
        let span = max_span.min(n);
        let inv_span = 1.0 / span as f32;
        for k in 0..span {
            let ramp = req_gain + (1.0 - req_gain) * (k as f32 * inv_span);
            let idx = (anchor_pos + n - k) % n;
            if ramp < self.env_buf[idx] {
                self.env_buf[idx] = ramp;
            }
        }
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2]) -> [f32; 2] {
        let n = Self::LOOKAHEAD_FRAMES;
        let delayed = self.delay_buf[self.pos];
        let scheduled_gain = self.env_buf[self.pos];

        self.delay_buf[self.pos] = input;
        self.env_buf[self.pos] = 1.0;

        // 4-tap cubic midpoint estimation between hist[1] (pos - 2) and hist[2] (pos - 1)
        let mid_l =
            0.5625 * (self.hist[1][0] + self.hist[2][0]) - 0.0625 * (self.hist[0][0] + input[0]);
        let mid_r =
            0.5625 * (self.hist[1][1] + self.hist[2][1]) - 0.0625 * (self.hist[0][1] + input[1]);
        self.hist[0] = self.hist[1];
        self.hist[1] = self.hist[2];
        self.hist[2] = input;

        let mid_peak = abs_f32(mid_l).max(abs_f32(mid_r));
        if mid_peak > self.ceiling_lin {
            let mid_gain = self.ceiling_lin / mid_peak;
            let p1 = (self.pos + n - 1) % n;
            let p2 = (self.pos + n - 2) % n;
            self.schedule_ramp(p1, n - 1, mid_gain);
            self.schedule_ramp(p2, n - 2, mid_gain);
        }

        let sample_peak = abs_f32(input[0]).max(abs_f32(input[1]));
        if sample_peak > self.ceiling_lin {
            let req_gain = self.ceiling_lin / sample_peak;
            self.schedule_ramp(self.pos, n, req_gain);
        }

        if scheduled_gain < self.gain_env {
            self.gain_env = scheduled_gain;
        } else {
            self.gain_env =
                scheduled_gain + self.release_coeff * (self.gain_env - scheduled_gain);
        }

        let applied_gain = self.gain_env.min(scheduled_gain);
        self.current_gr_db = if applied_gain < 1.0 {
            (-20.0 * log10_f32(applied_gain.max(1e-6))).max(0.0)
        } else {
            0.0
        };

        self.pos = (self.pos + 1) % n;
        [delayed[0] * applied_gain, delayed[1] * applied_gain]
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

/// Complete master finishing chain:
/// Input Trim (`-12..+18 dB`) -> 3-Band Mastering EQ (120 Hz Low Shelf / 500 Hz Body Bell / 8 kHz Air Shelf)
/// -> Soft-Knee Glue Bus Compressor -> Mid/Side `stereo_width` (`0.0..=2.0`) -> `LookaheadLimiter`.
#[derive(Clone, Debug)]
pub struct MasteringProcessor {
    trim_db: f32,
    trim_lin: f32,
    low_shelf_db: f32,
    body_bell_db: f32,
    air_shelf_db: f32,
    low_shelf: Biquad,
    body_bell: Biquad,
    air_shelf: Biquad,
    glue_enabled: bool,
    glue_compressor: SidechainCompressor,
    stereo_width: f32,
    limiter: LookaheadLimiter,
    pub(crate) processed_until: Option<usize>,
}

impl MasteringProcessor {
    pub fn new() -> Result<Self, Error> {
        let glue_compressor = SidechainCompressor::new(-12.0, 2.0, 10.0, 120.0, 0.0)
            .map_err(|_| Error::Dynamics)?;
        let limiter = LookaheadLimiter::default_master()?;

        Ok(Self {
            trim_db: 0.0,
            trim_lin: 1.0,
            low_shelf_db: 0.0,
            body_bell_db: 0.0,
            air_shelf_db: 0.0,
            low_shelf: Biquad::default(),
            body_bell: Biquad::default(),
            air_shelf: Biquad::default(),
            glue_enabled: true,
            glue_compressor,
            stereo_width: 1.0,
            limiter,
            processed_until: None,
        })
    }

    pub fn set_trim_db(&mut self, trim_db: f32) -> Result<(), Error> {
        if !trim_db.is_finite() || !(-12.0..=18.0).contains(&trim_db) {
            return Err(Error::Gain);
        }
        self.trim_db = trim_db;
        self.trim_lin = powf_f32(10.0, trim_db / 20.0);
        Ok(())
    }

    pub fn trim_db(&self) -> f32 {
        self.trim_db
    }

    pub fn set_eq_gains(&mut self, low_shelf_db: f32, body_bell_db: f32, air_shelf_db: f32) -> Result<(), Error> {
        for g in [low_shelf_db, body_bell_db, air_shelf_db] {
            if !g.is_finite() || !(-12.0..=12.0).contains(&g) {
                return Err(Error::Gain);
            }
        }
        self.low_shelf_db = low_shelf_db;
        self.body_bell_db = body_bell_db;
        self.air_shelf_db = air_shelf_db;
        self.low_shelf
            .configure(BandKind::LowShelf, 120.0, low_shelf_db, 0.707);
        self.body_bell
            .configure(BandKind::Bell, 500.0, body_bell_db, 0.8);
        self.air_shelf
            .configure(BandKind::HighShelf, 8000.0, air_shelf_db, 0.707);
        Ok(())
    }

    pub fn eq_gains(&self) -> (f32, f32, f32) {
        (self.low_shelf_db, self.body_bell_db, self.air_shelf_db)
    }

    pub fn set_glue_enabled(&mut self, enabled: bool) {
        self.glue_enabled = enabled;
    }

    pub fn set_glue_compressor(
        &mut self,
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    ) -> Result<(), Error> {
        self.glue_compressor
            .set_parameters(threshold_db, ratio, attack_ms, release_ms, makeup_db)
            .map_err(|_| Error::Dynamics)
    }

    pub fn set_stereo_width(&mut self, width: f32) -> Result<(), Error> {
        if !width.is_finite() || !(0.0..=2.0).contains(&width) {
            return Err(Error::Width);
        }
        self.stereo_width = width;
        Ok(())
    }

    pub fn stereo_width(&self) -> f32 {
        self.stereo_width
    }

    pub fn limiter(&self) -> &LookaheadLimiter {
        &self.limiter
    }

    pub fn limiter_mut(&mut self) -> &mut LookaheadLimiter {
        &mut self.limiter
    }

    pub fn glue_compressor(&self) -> &SidechainCompressor {
        &self.glue_compressor
    }

    pub fn latency_frames(&self) -> usize {
        self.limiter.latency_frames()
    }

    pub fn gain_reduction_db(&self) -> f32 {
        let glue = if self.glue_enabled {
            self.glue_compressor.gain_reduction_db()
        } else {
            0.0
        };
        glue + self.limiter.gain_reduction_db()
    }

    pub fn reset(&mut self) {
        self.low_shelf.reset();
        self.body_bell.reset();
        self.air_shelf.reset();
        self.glue_compressor.reset();
        self.limiter.reset();
        self.processed_until = None;
    }

    #[inline]
    pub fn process_stereo_f32(&mut self, input: [f32; 2]) -> [f32; 2] {
        let mut x = [input[0] * self.trim_lin, input[1] * self.trim_lin];
        x = self.low_shelf.process(x);
        x = self.body_bell.process(x);
        x = self.air_shelf.process(x);
        if self.glue_enabled {
            x = self.glue_compressor.process_stereo_f32(x, None);
        }
        if abs_f32(self.stereo_width - 1.0) > 1e-5 {
            let mid = 0.5 * (x[0] + x[1]);
            let side = 0.5 * (x[0] - x[1]) * self.stereo_width;
            x = [mid + side, mid - side];
        }
        self.limiter.process_stereo_f32(x)
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
