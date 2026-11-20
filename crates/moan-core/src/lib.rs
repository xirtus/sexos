// SPDX-License-Identifier: GPL-3.0-only
//! Moan's first engine slice. All positions and fades are stereo PCM frames at
//! 48 kHz (Bell's capture rate). Sources must be converted before insertion.
//! Clips borrow immutable audio; cutting never copies or changes source samples.
//! No allocation in render: the caller supplies a block and the playhead.
#![no_std]
extern crate alloc;
pub mod arp;
pub mod audio_to_midi;
pub mod automation;
pub mod dsp_math;
pub mod effects;
pub mod interchange;
pub mod loudness;
pub mod midi;
pub mod phonemes;
pub mod pronounce;
pub mod stretch;
pub mod studio_presets;
pub mod vocal;
pub mod voice_cleanup;
pub mod warp;
pub mod wavetable;
use alloc::{sync::Arc, vec::Vec};

#[derive(Clone, Debug, PartialEq)]
pub struct VocalClip {
    pub text: alloc::string::String,
    pub params: vocal::VoiceParams,
    pub spans: Vec<vocal::PhraseSpan>,
}

#[derive(Clone, Debug)]
enum Source<'a> {
    Borrowed(&'a [Frame]),
    Owned(Arc<[Frame]>),
    Buffer(Arc<Vec<Frame>>),
    Midi {
        pattern: Arc<midi::Pattern>,
        pcm: Arc<Vec<Frame>>,
    },
    Vocal {
        utterance: Arc<VocalClip>,
        pcm: Arc<Vec<Frame>>,
    },
}
impl Source<'_> {
    fn frames(&self) -> &[Frame] {
        match self {
            Self::Borrowed(v) => v,
            Self::Owned(v) => v,
            Self::Buffer(v) => v,
            Self::Midi { pcm, .. } => pcm,
            Self::Vocal { pcm, .. } => pcm,
        }
    }
}
pub const RATE: u32 = 48_000;
pub type Frame = [i16; 2];

#[derive(Clone, Debug)]
pub struct Clip<'a> {
    source: Source<'a>,
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
    Memory,
}
impl<'a> Clip<'a> {
    pub fn new(source: &'a [Frame], start: usize) -> Result<Self, Error> {
        start.checked_add(source.len()).ok_or(Error::Range)?;
        Ok(Self {
            source: Source::Borrowed(source),
            offset: 0,
            len: source.len(),
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    /// Shared immutable owned PCM for editable projects without self-references.
    pub fn owned(source: Arc<[Frame]>, start: usize) -> Result<Clip<'static>, Error> {
        start.checked_add(source.len()).ok_or(Error::Range)?;
        Ok(Clip {
            len: source.len(),
            source: Source::Owned(source),
            offset: 0,
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    /// Keep a completed recording/render buffer without copying its PCM into a
    /// second large allocation. Undo snapshots share this immutable allocation.
    pub fn buffer(source: Arc<Vec<Frame>>, start: usize) -> Result<Clip<'static>, Error> {
        start.checked_add(source.len()).ok_or(Error::Range)?;
        Ok(Clip {
            len: source.len(),
            source: Source::Buffer(source),
            offset: 0,
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    /// Editable MIDI source with an immutable audio cache, built outside render.
    pub fn midi(pattern: midi::Pattern, start: usize) -> Result<Clip<'static>, midi::Error> {
        let pcm = pattern.bake()?;
        start.checked_add(pcm.len()).ok_or(midi::Error::Range)?;
        Ok(Clip {
            len: pcm.len(),
            source: Source::Midi {
                pattern: Arc::new(pattern),
                pcm,
            },
            offset: 0,
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    pub fn midi_pattern(&self) -> Option<&midi::Pattern> {
        match &self.source {
            Source::Midi { pattern, .. } => Some(pattern),
            _ => None,
        }
    }
    /// Synthesize a vocal clip from lyrics and VoiceParams, storing source + karaoke spans.
    pub fn vocal(
        text: &str,
        params: vocal::VoiceParams,
        start: usize,
    ) -> Result<Clip<'static>, vocal::Error> {
        let (pcm, spans) = vocal::synthesize(text, &params)?;
        start.checked_add(pcm.len()).ok_or(vocal::Error::Range)?;
        let utterance = Arc::new(VocalClip {
            text: alloc::string::String::from(text),
            params,
            spans,
        });
        Ok(Clip {
            len: pcm.len(),
            source: Source::Vocal { utterance, pcm },
            offset: 0,
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    /// Construct a vocal clip from an existing VocalClip and its pre-baked PCM.
    pub fn vocal_with_pcm(
        utterance: VocalClip,
        pcm: Arc<Vec<Frame>>,
        start: usize,
    ) -> Result<Clip<'static>, vocal::Error> {
        start.checked_add(pcm.len()).ok_or(vocal::Error::Range)?;
        Ok(Clip {
            len: pcm.len(),
            source: Source::Vocal {
                utterance: Arc::new(utterance),
                pcm,
            },
            offset: 0,
            start,
            gain: 100,
            fade_in: 0,
            fade_out: 0,
        })
    }
    pub fn vocal_clip(&self) -> Option<&VocalClip> {
        match &self.source {
            Source::Vocal { utterance, .. } => Some(utterance),
            _ => None,
        }
    }
    /// Share a rebaked source while retaining every clip's trim, position and fades.
    pub fn replace_midi_source(&mut self, source: &Clip<'_>) -> Result<(), midi::Error> {
        if self.midi_pattern().is_none()
            || source.midi_pattern().is_none()
            || self.source_frames().len() != source.source_frames().len()
        {
            return Err(midi::Error::Range);
        }
        if let Source::Midi { pattern, pcm } = &source.source {
            self.source = Source::Midi {
                pattern: pattern.clone(),
                pcm: pcm.clone(),
            };
            Ok(())
        } else {
            Err(midi::Error::Range)
        }
    }
    /// Immutable source view, shared by project clips; never mutable audio.
    pub fn source_frames(&self) -> &[Frame] {
        self.source.frames()
    }
    pub fn source_offset(&self) -> usize {
        self.offset
    }
    pub fn fades(&self) -> (usize, usize) {
        (self.fade_in, self.fade_out)
    }
    pub fn gain(&self) -> u16 {
        self.gain
    }
    pub fn move_to(&mut self, start: usize) -> Result<(), Error> {
        start.checked_add(self.len).ok_or(Error::Range)?;
        self.start = start;
        Ok(())
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
        self.source.frames()[self.offset + i].map(|v| v as i64 * scale / 65536)
    }
}

#[derive(Clone)]
pub struct Track<'a> {
    pub effects: effects::Rack,
    processed_until: Option<usize>,
    pub clips: Vec<Clip<'a>>,
    volume: u16,
    pan: i16,
    pub muted: bool,
    pub solo: bool,
    pub distortion: Option<effects::Distortion>,
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
            distortion: None,
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
        if let Some(dist) = &mut self.distortion {
            dist.reset();
        }
        self.processed_until = None;
    }
    pub fn volume(&self) -> u16 {
        self.volume
    }
    pub fn pan(&self) -> i16 {
        self.pan
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
    /// Process a buffer through the track's distortion, effects rack, volume, and pan.
    pub fn process(&mut self, frames: &mut [Frame]) {
        for f in frames {
            let mut sample = f.map(|v| v as i64);
            if let Some(dist) = &mut self.distortion {
                sample = dist.frame(sample);
            }
            sample = self.effects.frame(sample);
            let balance = [100 - self.pan.max(0) as i64, 100 + self.pan.min(0) as i64];
            for c in 0..2 {
                sample[c] = sample[c]
                    .saturating_mul(self.volume as i64)
                    .saturating_mul(balance[c])
                    / 10000;
            }
            *f = sample.map(|v| v.clamp(i16::MIN as i64, i16::MAX as i64) as i16);
        }
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
    render_processed_observed(tracks, start, out, |_, _| {});
}
/// Tap the actual post-effects/post-fader track samples once, without a second DSP
/// pass or allocation. The caller may feed a preallocated meter; muted tracks tap silence.
pub fn render_processed_observed(
    tracks: &mut [Track<'_>],
    start: usize,
    out: &mut [Frame],
    sample: impl FnMut(usize, Frame),
) {
    render_processed_observed_master(tracks, start, out, 100, sample);
}
/// Scale the wide accumulated bus before its single final i16 saturation.
/// Unity preserves the existing renderer and track tap semantics exactly.
pub fn render_processed_observed_master(
    tracks: &mut [Track<'_>],
    start: usize,
    out: &mut [Frame],
    master_percent: u16,
    mut sample: impl FnMut(usize, Frame),
) {
    if out.is_empty() {
        return;
    }
    let solo = tracks.iter().any(|t| t.solo && !t.muted);
    for t in tracks.iter_mut() {
        if t.processed_until != Some(start) {
            t.effects.reset();
            if let Some(dist) = &mut t.distortion {
                dist.reset();
            }
        }
    }
    for (i, dest) in out.iter_mut().enumerate() {
        *dest = [0; 2];
        let Some(at) = start.checked_add(i) else {
            continue;
        };
        let mut mix = [0i64; 2];
        for (index, t) in tracks.iter_mut().enumerate() {
            let mut sum = [0i64; 2];
            for clip in &t.clips {
                let f = clip.sample(at);
                for c in 0..2 {
                    sum[c] = sum[c].saturating_add(f[c]);
                }
            }
            if let Some(dist) = &mut t.distortion {
                sum = dist.frame(sum);
            }
            let f = t.effects.frame(sum);
            if t.muted || (solo && !t.solo) {
                sample(index, [0; 2]);
                continue;
            }
            let balance = [100 - t.pan.max(0) as i64, 100 + t.pan.min(0) as i64];
            let mut post = [0i64; 2];
            for c in 0..2 {
                post[c] = f[c]
                    .saturating_mul(t.volume as i64)
                    .saturating_mul(balance[c])
                    / 10000;
                mix[c] = mix[c].saturating_add(post[c]);
            }
            sample(
                index,
                post.map(|v| v.clamp(i16::MIN as i64, i16::MAX as i64) as i16),
            );
        }
        *dest = mix.map(|v| {
            (v.saturating_mul(master_percent as i64) / 100).clamp(i16::MIN as i64, i16::MAX as i64)
                as i16
        });
    }
    for t in tracks {
        t.processed_until = start.checked_add(out.len());
    }
}
