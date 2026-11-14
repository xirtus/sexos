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
