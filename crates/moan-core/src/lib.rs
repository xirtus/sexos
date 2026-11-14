// SPDX-License-Identifier: GPL-3.0-only
//! Moan's first engine slice. All positions and fades are stereo PCM frames at
//! 48 kHz (Bell's capture rate). Sources must be converted before insertion.
//! Clips borrow immutable audio; cutting never copies or changes source samples.
//! No allocation in render: the caller supplies a block and the playhead.
#![no_std]
extern crate alloc;
pub mod effects;
use alloc::vec::Vec;
pub const RATE: u32 = 48_000;
pub type Frame = [i16; 2];

#[derive(Clone, Debug)]
pub struct Clip<'a> {
    source: &'a [Frame],
    offset: usize,
    len: usize,
    start: usize,
    gain: u16,
    fade_in: usize,
    fade_out: usize,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Range,
    Gain,
    Pan,
}
impl<'a> Clip<'a> {
    pub fn new(source: &'a [Frame], start: usize) -> Result<Self, Error> {
        start.checked_add(source.len()).ok_or(Error::Range)?;
        Ok(Self {
            source,
            offset: 0,
            len: source.len(),
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    pub fn start(&self) -> usize {
        self.start
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn end(&self) -> usize {
        self.start + self.len
    }
    /// 0..400 percent, unity at 100.
    pub fn set_gain(&mut self, percent: u16) -> Result<(), Error> {
        if percent > 400 {
            return Err(Error::Gain);
        }
        self.gain = percent;
        Ok(())
    }
    /// Fade lengths may overlap; their linear envelopes multiply. A one-frame
    /// fade silences that endpoint. Lengths larger than the clip are refused.
    pub fn set_fades(&mut self, incoming: usize, outgoing: usize) -> Result<(), Error> {
        if incoming > self.len || outgoing > self.len {
            return Err(Error::Range);
        }
        self.fade_in = incoming;
        self.fade_out = outgoing;
        Ok(())
    }
    /// Keep a relative half-open range in place on the timeline. New cut edges
    /// start with no fades; gain is retained. The source is never modified.
    pub fn trim(&mut self, from: usize, to: usize) -> Result<(), Error> {
        if from > to || to > self.len {
            return Err(Error::Range);
        }
        self.offset += from;
        self.start += from;
        self.len = to - from;
        self.fade_in = 0;
        self.fade_out = 0;
        Ok(())
    }
    /// Split strictly inside the clip. Existing outer fades stay on their side,
    /// clamped to the new length; the new inner edges have no fades.
    pub fn split(&mut self, at: usize) -> Result<Self, Error> {
        if at == 0 || at >= self.len {
            return Err(Error::Range);
        }
        let mut right = self.clone();
        right.offset += at;
        right.start += at;
        right.len -= at;
        right.fade_in = 0;
        right.fade_out = right.fade_out.min(right.len);
        self.len = at;
        self.fade_in = self.fade_in.min(at);
        self.fade_out = 0;
        Ok(right)
    }
    fn sample(&self, at: usize) -> [i64; 2] {
        if at < self.start || at >= self.end() {
            return [0; 2];
        }
        let i = at - self.start;
        let mut scale = self.gain as i64 * 65536 / 100;
        for (pos, fade) in [(i, self.fade_in), (self.len - 1 - i, self.fade_out)] {
            if fade != 0 && pos < fade {
                // u128 prevents long timelines overflowing a fixed-point envelope.
                let envelope = if fade == 1 {
                    0
                } else {
                    (pos as u128 * 65536 / (fade - 1) as u128) as i64
                };
                scale = scale * envelope / 65536;
            }
        }
        self.source[self.offset + i].map(|v| v as i64 * scale / 65536)
    }
}

pub struct Track<'a> {
    pub effects: effects::Rack,
    processed_until: Option<usize>,
    pub clips: Vec<Clip<'a>>,
    volume: u16,
    pan: i16,
    pub muted: bool,
    pub solo: bool,
}
impl Default for Track<'_> {
    fn default() -> Self {
        Self {
            clips: Vec::new(),
            effects: effects::Rack::default(),
            processed_until: None,
            volume: 100,
            pan: 0,
            muted: false,
            solo: false,
        }
    }
}
impl<'a> Track<'a> {
    pub fn new() -> Self {
        Self::default()
    }
    /// Flush tails after edits, changing projects, or restarting transport.
    pub fn reset_effects(&mut self) {
        self.effects.reset();
        self.processed_until = None;
    }
    pub fn set_volume(&mut self, percent: u16) -> Result<(), Error> {
        if percent > 400 {
            return Err(Error::Gain);
        }
        self.volume = percent;
        Ok(())
    }
    /// Stereo balance: -100 keeps only left; +100 keeps only right.
    /// Centre preserves both channels at unity (no crossfeed).
    pub fn set_pan(&mut self, pan: i16) -> Result<(), Error> {
        if !(-100..=100).contains(&pan) {
            return Err(Error::Pan);
        }
        self.pan = pan;
        Ok(())
    }
}

/// Render a dry block (effects bypassed), including silence, with one final saturating PCM
/// conversion after all clips/tracks are summed. Splitting blocks does not change
/// samples. Positions beyond usize's timeline limit render silence.
pub fn render(tracks: &[Track<'_>], start: usize, out: &mut [Frame]) {
    let solo = tracks.iter().any(|t| t.solo && !t.muted);
    for (i, dest) in out.iter_mut().enumerate() {
        *dest = [0; 2];
        let Some(at) = start.checked_add(i) else {
            continue;
        };
        let mut mix = [0i64; 2];
        for track in tracks {
            if track.muted || (solo && !track.solo) {
                continue;
            }
            let balance = [100 - track.pan.max(0) as i64, 100 + track.pan.min(0) as i64];
            let mut sum = [0i64; 2];
            for clip in &track.clips {
                let f = clip.sample(at);
                for c in 0..2 {
                    sum[c] = sum[c].saturating_add(f[c]);
                }
            }
            for c in 0..2 {
                let value = sum[c]
                    .saturating_mul(track.volume as i64)
                    .saturating_mul(balance[c])
                    / 10000;
                mix[c] = mix[c].saturating_add(value);
            }
        }
        *dest = mix.map(|v| v.clamp(i16::MIN as i64, i16::MAX as i64) as i16);
    }
}

/// Stateful per-track effects, before volume/balance and final summing. Sequential
/// blocks preserve tails; a discontinuous start resets all filter/delay state.
/// Muted/non-solo tracks still advance their DSP but do not enter the output.
/// Pausing means making no calls. Empty blocks do not alter DSP or position.
/// This processes indefinitely beyond clip ends so reverberation can decay.
pub fn render_processed(tracks: &mut [Track<'_>], start: usize, out: &mut [Frame]) {
    if out.is_empty() {
        return;
    }
    let solo = tracks.iter().any(|t| t.solo && !t.muted);
    for t in tracks.iter_mut() {
        if t.processed_until != Some(start) {
            t.effects.reset();
        }
    }
    for (i, dest) in out.iter_mut().enumerate() {
        *dest = [0; 2];
        let Some(at) = start.checked_add(i) else {
            continue;
        };
        let mut mix = [0i64; 2];
        for t in tracks.iter_mut() {
            let mut sum = [0i64; 2];
            for clip in &t.clips {
                let f = clip.sample(at);
                for c in 0..2 {
                    sum[c] = sum[c].saturating_add(f[c]);
                }
            }
            let f = t.effects.frame(sum);
            if t.muted || (solo && !t.solo) {
                continue;
            }
            let balance = [100 - t.pan.max(0) as i64, 100 + t.pan.min(0) as i64];
            for c in 0..2 {
                mix[c] = mix[c].saturating_add(
                    f[c].saturating_mul(t.volume as i64)
                        .saturating_mul(balance[c])
                        / 10000,
                );
            }
        }
        *dest = mix.map(|v| v.clamp(i16::MIN as i64, i16::MAX as i64) as i16);
    }
    for t in tracks {
        t.processed_until = start.checked_add(out.len());
    }
}
