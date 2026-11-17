// SPDX-License-Identifier: GPL-3.0-only
//! Bounded integer DSP at 48 kHz. No allocations after configuration.
//! Order: three-band EQ, stereo-linked peak compressor, Schroeder reverb.
use crate::Frame;
use alloc::{vec, vec::Vec};

const Q: i64 = 65536;
// Headroom for intermediate signals, well above any i16 track's legal gain.
const LIMIT: i64 = 1 << 26;
fn bound(v: i64) -> i64 {
    v.clamp(-LIMIT, LIMIT)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Gain,
    Threshold,
    Ratio,
    Timing,
    Reverb,
}

/// Three bands split by one-pole lowpasses at nominal 200 and 4000 Hz.
/// The high and mid bands are residuals, so unity gains reconstruct exactly.
#[derive(Clone)]
pub struct Equalizer {
    gains: [u16; 3],
    low: [i64; 2],
    upper: [i64; 2],
}
impl Equalizer {
    pub fn gains(&self) -> [u16; 3] {
        self.gains
    }
    pub fn new(low: u16, mid: u16, high: u16) -> Result<Self, Error> {
        if [low, mid, high].iter().any(|g| *g > 400) {
            return Err(Error::Gain);
        }
        Ok(Self {
            gains: [low, mid, high],
            low: [0; 2],
            upper: [0; 2],
        })
    }
    fn frame(&mut self, input: [i64; 2]) -> [i64; 2] {
        let mut out = [0; 2];
        for c in 0..2 {
            let x = input[c] * Q;
            // alpha = 2*pi*f / (RATE + 2*pi*f), rounded to Q16.
            self.low[c] += (x - self.low[c]) * 1672 / Q;
            self.upper[c] += (x - self.upper[c]) * 22522 / Q;
            let bands = [self.low[c], self.upper[c] - self.low[c], x - self.upper[c]];
            out[c] = bound(
                bands
                    .iter()
                    .zip(self.gains)
                    .map(|(v, g)| v * g as i64)
                    .sum::<i64>()
                    / (100 * Q),
            );
        }
        out
    }
    fn reset(&mut self) {
        self.low = [0; 2];
        self.upper = [0; 2];
    }
    /// Returns frequency-response curve samples in Q8 dB (256 = 1.0 dB) across
    /// log-spaced frequencies from 20 Hz to 20,000 Hz.
    pub fn response_curve(&self, out_db_q8: &mut [i16]) {
        use crate::dsp_math::{cos_f32, exp2_f32, log10_f32, sin_f32, sqrt_f32, PI};
        if out_db_q8.is_empty() {
            return;
        }
        let n = out_db_q8.len();
        let a1 = 1672.0 / 65536.0;
        let a2 = 22522.0 / 65536.0;
        let c1 = 1.0 - a1;
        let c2 = 1.0 - a2;

        let g0 = self.gains[0] as f32 / 100.0;
        let g1 = self.gains[1] as f32 / 100.0;
        let g2 = self.gains[2] as f32 / 100.0;

        for (i, p) in out_db_q8.iter_mut().enumerate() {
            let f = if n == 1 {
                1000.0
            } else {
                20.0 * exp2_f32((i as f32 / (n - 1) as f32) * 9.96578428)
            };
            let w = 2.0 * PI * f / (crate::RATE as f32);
            let cos_w = cos_f32(w);
            let sin_w = sin_f32(w);

            let d1_denom = (1.0 - c1 * cos_w) * (1.0 - c1 * cos_w) + (c1 * sin_w) * (c1 * sin_w);
            let h1_re = a1 * (1.0 - c1 * cos_w) / d1_denom;
            let h1_im = -a1 * (c1 * sin_w) / d1_denom;

            let d2_denom = (1.0 - c2 * cos_w) * (1.0 - c2 * cos_w) + (c2 * sin_w) * (c2 * sin_w);
            let h2_re = a2 * (1.0 - c2 * cos_w) / d2_denom;
            let h2_im = -a2 * (c2 * sin_w) / d2_denom;

            let re = g2 + (g0 - g1) * h1_re + (g1 - g2) * h2_re;
            let im = (g0 - g1) * h1_im + (g1 - g2) * h2_im;
            let mag = sqrt_f32(re * re + im * im).max(1e-6);
            let db = 20.0 * log10_f32(mag);
            *p = (db * 256.0).clamp(-32768.0, 32767.0) as i16;
        }
    }
}

/// Stereo-linked feed-forward peak compression, no makeup gain or lookahead.
/// Attack/release are smoothing divisors in frames, not brick-wall guarantees.
/// One-frame attack responds immediately; slower attack lets transients through.
#[derive(Clone)]
pub struct Compressor {
    threshold: i64,
    ratio: i64,
    attack: i64,
    release: i64,
    envelope: i64,
}
impl Compressor {
    pub fn parameters(&self) -> (u16, u16, u32, u32) {
        (
            (self.threshold / Q) as u16,
            self.ratio as u16,
            self.attack as u32,
            self.release as u32,
        )
    }
    pub fn new(
        threshold: u16,
        ratio: u16,
        attack_frames: u32,
        release_frames: u32,
    ) -> Result<Self, Error> {
        if threshold == 0 || threshold > 32767 {
            return Err(Error::Threshold);
        }
        if !(1..=20).contains(&ratio) {
            return Err(Error::Ratio);
        }
        if attack_frames == 0
            || release_frames == 0
            || attack_frames > 480000
            || release_frames > 480000
        {
            return Err(Error::Timing);
        }
        Ok(Self {
            threshold: threshold as i64 * Q,
            ratio: ratio as i64,
            attack: attack_frames as i64,
            release: release_frames as i64,
            envelope: 0,
        })
    }
    fn frame(&mut self, input: [i64; 2]) -> [i64; 2] {
        let peak = input[0].abs().max(input[1].abs()) * Q;
        let divisor = if peak > self.envelope {
            self.attack
        } else {
            self.release
        };
        self.envelope += (peak - self.envelope) / divisor;
        if self.envelope <= self.threshold {
            return input;
        }
        let compressed = self.threshold + (self.envelope - self.threshold) / self.ratio;
        let gain = compressed * Q / self.envelope;
        input.map(|v| v * gain / Q)
    }
    fn reset(&mut self) {
        self.envelope = 0;
    }
}

#[derive(Clone)]
struct Delay {
    samples: Vec<[i64; 2]>,
    at: usize,
}
impl Delay {
    fn new(len: usize) -> Self {
        Self {
            samples: vec![[0; 2]; len],
            at: 0,
        }
    }
    fn exchange(&mut self, value: [i64; 2]) -> [i64; 2] {
        let old = self.samples[self.at];
        self.samples[self.at] = value;
        self.at = (self.at + 1) % self.samples.len();
        old
    }
    fn reset(&mut self) {
        self.samples.fill([0; 2]);
        self.at = 0;
    }
}

/// Four parallel feedback combs followed by two serial allpasses. The delay
/// lengths are fixed at 48 kHz. Feedback 0..90% controls decay, wet 0..100% is
/// added to the dry signal. Zero wet still advances the network's state.
#[derive(Clone)]
pub struct Reverb {
    combs: [Delay; 4],
    diffusers: [Delay; 2],
    feedback: i64,
    wet: i64,
}
impl Reverb {
    pub fn parameters(&self) -> (u16, u16) {
        (self.feedback as u16, self.wet as u16)
    }
    pub fn new(feedback: u16, wet: u16) -> Result<Self, Error> {
        if feedback > 90 || wet > 100 {
            return Err(Error::Reverb);
        }
        Ok(Self {
            combs: [1429, 1789, 2081, 2333].map(Delay::new),
            diffusers: [211, 71].map(Delay::new),
            feedback: feedback as i64,
            wet: wet as i64,
        })
    }
    fn frame(&mut self, input: [i64; 2]) -> [i64; 2] {
        let mut diffuse = [0; 2];
        for delay in &mut self.combs {
            let old = delay.samples[delay.at];
            delay.exchange(core::array::from_fn(|c| {
                bound(input[c] + old[c] * self.feedback / 100)
            }));
            for c in 0..2 {
                diffuse[c] += old[c];
            }
        }
        diffuse = diffuse.map(|v| v / 4);
        for delay in &mut self.diffusers {
            let old = delay.samples[delay.at];
            let output: [i64; 2] = core::array::from_fn(|c| bound(old[c] - diffuse[c] / 2));
            delay.exchange(core::array::from_fn(|c| bound(diffuse[c] + output[c] / 2)));
            diffuse = output;
        }
        core::array::from_fn(|c| bound(input[c] + diffuse[c] * self.wet / 100))
    }
    fn reset(&mut self) {
        for delay in self.combs.iter_mut().chain(&mut self.diffusers) {
            delay.reset();
        }
    }
}

/// An empty rack is bit-exact bypass. Changing a stage replaces its history.
/// A new reverb allocates about 124 KiB; processing and reset never allocate.
#[derive(Clone, Default)]
pub struct Rack {
    pub eq: Option<Equalizer>,
    pub compressor: Option<Compressor>,
    pub reverb: Option<Reverb>,
}
impl Rack {
    pub fn reset(&mut self) {
        if let Some(eq) = &mut self.eq {
            eq.reset();
        }
        if let Some(compressor) = &mut self.compressor {
            compressor.reset();
        }
        if let Some(reverb) = &mut self.reverb {
            reverb.reset();
        }
    }
    pub(crate) fn frame(&mut self, input: [i64; 2]) -> [i64; 2] {
        // Preserve wide sums on the bypass path, including cancellation across
        // tracks. Configured DSP uses bounded headroom to keep arithmetic safe.
        if self.eq.is_none() && self.compressor.is_none() && self.reverb.is_none() {
            return input;
        }
        let mut frame = input.map(bound);
        if let Some(eq) = &mut self.eq {
            frame = eq.frame(frame);
        }
        if let Some(compressor) = &mut self.compressor {
            frame = compressor.frame(frame);
        }
        if let Some(reverb) = &mut self.reverb {
            frame = reverb.frame(frame);
        }
        frame
    }
    /// Standalone processing, useful for capture or a master bus. Call reset
    /// explicitly when a new stream begins; chunk boundaries preserve state.
    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            *f = self
                .frame(f.map(|v| v as i64))
                .map(|v| v.clamp(i16::MIN as i64, i16::MAX as i64) as i16);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistortionMode {
    Symmetric,
    Asymmetric,
    HardClip,
    Foldback,
}

/// Multi-mode studio distortion with pre-drive, tilt, bias, cutoff filter, and dry/wet mix.
#[derive(Clone)]
pub struct Distortion {
    drive: u16,        // 0..400% (100 = unity)
    tilt: i16,         // -100..100
    bias: i16,         // -100..100
    mix: u16,          // 0..100%
    cutoff: u16,       // 200..20000 Hz
    mode: DistortionMode,
    prev_lowpass: [i64; 2],
    prev_tilt: [i64; 2],
}

impl Distortion {
    pub fn new(
        drive: u16,
        tilt: i16,
        bias: i16,
        mix: u16,
        cutoff: u16,
        mode: DistortionMode,
    ) -> Result<Self, Error> {
        if drive > 400 || mix > 100 {
            return Err(Error::Gain);
        }
        if !(-100..=100).contains(&tilt) || !(-100..=100).contains(&bias) {
            return Err(Error::Gain);
        }
        if !(200..=20000).contains(&cutoff) {
            return Err(Error::Timing);
        }
        Ok(Self {
            drive,
            tilt,
            bias,
            mix,
            cutoff,
            mode,
            prev_lowpass: [0; 2],
            prev_tilt: [0; 2],
        })
    }

    pub fn parameters(&self) -> (u16, i16, i16, u16, u16, DistortionMode) {
        (
            self.drive,
            self.tilt,
            self.bias,
            self.mix,
            self.cutoff,
            self.mode,
        )
    }

    pub fn reset(&mut self) {
        self.prev_lowpass = [0; 2];
        self.prev_tilt = [0; 2];
    }

    pub fn shape_sample(&self, input: i64) -> i64 {
        let driven = input * self.drive as i64 / 100;
        let biased = driven + (self.bias as i64 * 16384 / 100);

        match self.mode {
            DistortionMode::Symmetric => {
                let abs_x = biased.abs();
                let denom = 32768 + abs_x;
                if denom == 0 {
                    0
                } else {
                    biased * 32768 / denom
                }
            }
            DistortionMode::Asymmetric => {
                if biased >= 0 {
                    let denom = 32768 + biased / 2;
                    if denom == 0 { 0 } else { biased * 32768 / denom }
                } else {
                    let abs_x = -biased;
                    let denom = 32768 + abs_x * 2;
                    if denom == 0 { 0 } else { biased * 32768 / denom }
                }
            }
            DistortionMode::HardClip => {
                biased.clamp(-24000, 24000)
            }
            DistortionMode::Foldback => {
                const T: i64 = 20000;
                let sign = if biased < 0 { -1 } else { 1 };
                let a = biased.abs();
                let folded = if a <= T {
                    a
                } else if a <= 3 * T {
                    2 * T - a
                } else {
                    a - 4 * T
                };
                sign * folded.clamp(-32768, 32767)
            }
        }
    }

    pub fn transfer_curve(&self, out: &mut [i16]) {
        if out.is_empty() {
            return;
        }
        let n = out.len();
        for (i, p) in out.iter_mut().enumerate() {
            let in_val = if n == 1 {
                0
            } else {
                -32768 + (i as i64 * 65535 / (n - 1) as i64)
            };
            let wet = self.shape_sample(in_val);
            let mixed = (in_val * (100 - self.mix as i64) + wet * self.mix as i64) / 100;
            *p = mixed.clamp(i16::MIN as i64, i16::MAX as i64) as i16;
        }
    }

    pub(crate) fn frame(&mut self, input: [i64; 2]) -> [i64; 2] {
        if self.drive == 100 && self.mix == 0 && self.bias == 0 {
            return input;
        }
        use crate::dsp_math::PI;
        let alpha = ((2.0 * PI * self.cutoff as f32) / (48000.0 + 2.0 * PI * self.cutoff as f32) * 65536.0) as i64;
        let mut out = [0i64; 2];
        for c in 0..2 {
            let shaped = self.shape_sample(input[c]);
            let tilted = shaped - (self.tilt as i64 * self.prev_tilt[c] / 200);
            self.prev_tilt[c] = shaped;
            self.prev_lowpass[c] += (tilted - self.prev_lowpass[c]) * alpha / 65536;
            let wet = self.prev_lowpass[c];
            out[c] = bound((input[c] * (100 - self.mix as i64) + wet * self.mix as i64) / 100);
        }
        out
    }
}

