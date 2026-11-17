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
pub mod compose;
pub mod device_presets;
pub mod dsp_math;
pub mod effects;
pub mod instruments;
pub mod kits;
pub mod mastering;
pub mod midi;
pub mod parametric_eq;
pub mod pattern;
pub mod phonemes;
pub mod pronounce;
pub mod smf;
pub mod studio_presets;
pub mod tape_fx;
pub mod time;
pub mod vocal;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendRoute {
    pub return_track: u8,
    pub send_pct: u16,
}

impl SendRoute {
    pub const fn new(return_track: u8, send_pct: u16) -> Self {
        Self {
            return_track,
            send_pct,
        }
    }

    pub const fn as_tuple(self) -> (u8, u16) {
        (self.return_track, self.send_pct)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidechainSource {
    pub key_track: u8,
}

impl SidechainSource {
    pub const fn new(key_track: u8) -> Self {
        Self { key_track }
    }
}

#[derive(Clone, Debug)]
pub struct MasterBus {
    pub volume_pct: u16,
    pub mastering: Option<mastering::MasteringProcessor>,
}

impl Default for MasterBus {
    fn default() -> Self {
        Self {
            volume_pct: 100,
            mastering: None,
        }
    }
}

impl MasterBus {
    pub fn new(volume_pct: u16, mastering: Option<mastering::MasteringProcessor>) -> Self {
        Self {
            volume_pct,
            mastering,
        }
    }

    pub fn reset(&mut self) {
        if let Some(m) = &mut self.mastering {
            m.reset();
        }
    }

    pub fn render(
        &mut self,
        tracks: &mut [Track<'_>],
        start: usize,
        out: &mut [Frame],
        tap: impl FnMut(usize, Frame, i16),
    ) {
        render_console_mix(
            tracks,
            start,
            out,
            self.volume_pct,
            self.mastering.as_mut(),
            tap,
        );
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
    pub parametric_eq: Option<parametric_eq::ParametricEq>,
    pub sc_compressor: Option<tape_fx::SidechainCompressor>,
    pub gate: Option<tape_fx::SidechainGate>,
    pub tape: Option<tape_fx::TapeMachine>,
    pub transient: Option<tape_fx::TransientShaper>,
    pub crusher: Option<tape_fx::BitCrusher>,
    pub chorus: Option<tape_fx::StereoChorus>,
    pub sidechain_from: Option<u8>,
    pub post_fader_send: Option<(u8, u16)>,
    pub output_bus: Option<u8>,
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
            parametric_eq: None,
            sc_compressor: None,
            gate: None,
            tape: None,
            transient: None,
            crusher: None,
            chorus: None,
            sidechain_from: None,
            post_fader_send: None,
            output_bus: None,
        }
    }
}
impl<'a> Track<'a> {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set_send_route(&mut self, route: Option<SendRoute>) {
        self.post_fader_send = route.map(SendRoute::as_tuple);
    }
    pub fn set_sidechain_source(&mut self, source: Option<SidechainSource>) {
        self.sidechain_from = source.map(|s| s.key_track);
    }
    fn has_pro_fx(&self) -> bool {
        self.parametric_eq.is_some()
            || self.sc_compressor.is_some()
            || self.gate.is_some()
            || self.tape.is_some()
            || self.transient.is_some()
            || self.crusher.is_some()
            || self.chorus.is_some()
    }
    /// Flush tails after edits, changing projects, or restarting transport.
    pub fn reset_effects(&mut self) {
        self.effects.reset();
        if let Some(dist) = &mut self.distortion {
            dist.reset();
        }
        if let Some(eq) = &mut self.parametric_eq {
            eq.reset();
        }
        if let Some(comp) = &mut self.sc_compressor {
            comp.reset();
        }
        if let Some(gate) = &mut self.gate {
            gate.reset();
        }
        if let Some(tape) = &mut self.tape {
            tape.reset();
        }
        if let Some(tr) = &mut self.transient {
            tr.reset();
        }
        if let Some(cr) = &mut self.crusher {
            cr.reset();
        }
        if let Some(ch) = &mut self.chorus {
            ch.reset();
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
    fn process_console_frame(
        &mut self,
        mut x: [f32; 2],
        key: Option<[f32; 2]>,
    ) -> ([f32; 2], f32) {
        let mut gr_db = 0.0f32;
        let external_gate = self.sidechain_from.is_some();

        // Self-keyed gate cleans noise before distortion, compression, and reverb.
        if !external_gate {
            if let Some(gate) = &mut self.gate {
                x = gate.process_stereo_f32(x, None);
                gr_db += gate.gain_reduction_db();
            }
        }
        if let Some(tr) = &mut self.transient {
            x = tr.process_stereo_f32(x);
        }
        if let Some(dist) = &mut self.distortion {
            let s = [
                dsp_math::round_f32(x[0] * 32768.0) as i64,
                dsp_math::round_f32(x[1] * 32768.0) as i64,
            ];
            let d = dist.frame(s);
            x = [d[0] as f32 / 32768.0, d[1] as f32 / 32768.0];
        }
        if let Some(cr) = &mut self.crusher {
            x = cr.process_stereo_f32(x);
        }
        if let Some(tp) = &mut self.tape {
            x = tp.process_stereo_f32(x);
        }
        if let Some(eq) = &mut self.parametric_eq {
            x = eq.process_stereo_f32(x);
        }
        if let Some(comp) = &mut self.sc_compressor {
            x = comp.process_stereo_f32(x, key);
            gr_db += comp.gain_reduction_db();
        }
        if let Some(ch) = &mut self.chorus {
            x = ch.process_stereo_f32(x);
        }
        if !self.effects.is_bypass() {
            let s = [
                dsp_math::round_f32(x[0] * 32768.0) as i64,
                dsp_math::round_f32(x[1] * 32768.0) as i64,
            ];
            let r = self.effects.frame(s);
            x = [r[0] as f32 / 32768.0, r[1] as f32 / 32768.0];
        }
        // Externally keyed gate runs post-reverb/rack to support gated reverb returns & rhythmic ducking.
        if external_gate {
            if let Some(gate) = &mut self.gate {
                x = gate.process_stereo_f32(x, key);
                gr_db += gate.gain_reduction_db();
            }
        }
        (x, gr_db)
    }
    /// Process a buffer through the track's insert FX, effects rack, volume, and pan.
    pub fn process(&mut self, frames: &mut [Frame]) {
        if !self.has_pro_fx() {
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
            return;
        }
        let balance = [100 - self.pan.max(0) as i64, 100 + self.pan.min(0) as i64];
        let scale = [
            self.volume as f32 * balance[0] as f32 / 10000.0,
            self.volume as f32 * balance[1] as f32 / 10000.0,
        ];
        for f in frames {
            let (pre, _) = self.process_console_frame(parametric_eq::frame_to_f32(*f), None);
            *f = parametric_eq::f32_to_frame([pre[0] * scale[0], pre[1] * scale[1]]);
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

const MAX_CONSOLE_TRACKS: usize = 64;

#[inline]
fn must_precede(tracks: &[Track<'_>], u: usize, v: usize) -> bool {
    if u == v {
        return false;
    }
    let n = tracks.len();
    if tracks[v].sidechain_from.map(|k| k as usize) == Some(u) {
        return true;
    }
    if let Some((dst, _)) = tracks[u].post_fader_send {
        if (dst as usize) == v && v < n {
            return true;
        }
    }
    if let Some(dst) = tracks[u].output_bus {
        if (dst as usize) == v && v < n {
            return true;
        }
    }
    false
}

/// Full studio console mix renderer with zero heap allocations during rendering.
/// Executes tracks in source -> bus/return dependency order with sidechain key tap,
/// post-fader aux sends, bus summing, master fader scaling, optional `MasteringProcessor`,
/// and per-track meter tap `(track_index, post_fader_frame, gr_db_q8)`.
pub fn render_console_mix(
    tracks: &mut [Track<'_>],
    start: usize,
    out: &mut [Frame],
    master_volume: u16,
    mut mastering: Option<&mut mastering::MasteringProcessor>,
    mut tap: impl FnMut(usize, Frame, i16),
) {
    if out.is_empty() {
        return;
    }

    let any_solo = tracks.iter().any(|t| t.solo && !t.muted);
    for t in tracks.iter_mut() {
        if t.processed_until != Some(start) {
            t.reset_effects();
        }
    }
    if let Some(m) = mastering.as_deref_mut() {
        if m.processed_until.is_some() && m.processed_until != Some(start) {
            m.reset();
        }
    }

    let n = tracks.len().min(MAX_CONSOLE_TRACKS);
    let mut order = [0usize; MAX_CONSOLE_TRACKS];
    let mut in_degree = [0u16; MAX_CONSOLE_TRACKS];
    let mut in_order = [false; MAX_CONSOLE_TRACKS];
    let mut queue = [0usize; MAX_CONSOLE_TRACKS];
    let mut q_head = 0usize;
    let mut q_tail = 0usize;

    for v in 0..n {
        let mut deg = 0u16;
        for u in 0..n {
            if must_precede(&tracks[..n], u, v) {
                deg += 1;
            }
        }
        in_degree[v] = deg;
        if deg == 0 {
            queue[q_tail] = v;
            q_tail += 1;
        }
    }

    let mut order_len = 0usize;
    while q_head < q_tail {
        let u = queue[q_head];
        q_head += 1;
        order[order_len] = u;
        order_len += 1;
        in_order[u] = true;

        for v in 0..n {
            if !in_order[v] && in_degree[v] > 0 && must_precede(&tracks[..n], u, v) {
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue[q_tail] = v;
                    q_tail += 1;
                }
            }
        }
    }

    // Fallback for any routing cycle so no track is ever dropped.
    if order_len < n {
        for v in 0..n {
            if !in_order[v] {
                order[order_len] = v;
                order_len += 1;
            }
        }
    }

    let master_scale = master_volume as f32 / 100.0;

    for (i, dest) in out.iter_mut().enumerate() {
        *dest = [0; 2];
        let Some(at) = start.checked_add(i) else {
            continue;
        };

        let mut bus_accum = [[0.0f32; 2]; MAX_CONSOLE_TRACKS];
        let mut sidechain_tap = [[0.0f32; 2]; MAX_CONSOLE_TRACKS];
        let mut tapped_frames = [[0i16; 2]; MAX_CONSOLE_TRACKS];
        let mut gr_q8 = [0i16; MAX_CONSOLE_TRACKS];
        let mut master_mix = [0.0f32; 2];

        for &idx in &order[..n] {
            let key_f32 = tracks[idx].sidechain_from.and_then(|k| {
                let ki = k as usize;
                if ki < n && ki != idx {
                    Some(sidechain_tap[ki])
                } else {
                    None
                }
            });

            let t = &mut tracks[idx];
            let mut clip_sum = [0i64; 2];
            for clip in &t.clips {
                let cf = clip.sample(at);
                clip_sum[0] = clip_sum[0].saturating_add(cf[0]);
                clip_sum[1] = clip_sum[1].saturating_add(cf[1]);
            }
            let track_in = [
                clip_sum[0] as f32 / 32768.0 + bus_accum[idx][0],
                clip_sum[1] as f32 / 32768.0 + bus_accum[idx][1],
            ];

            let (pre_fader, gr_db) = t.process_console_frame(track_in, key_f32);
            let balance = [100 - t.pan.max(0) as i64, 100 + t.pan.min(0) as i64];
            let post_fader = [
                pre_fader[0] * (t.volume as f32 * balance[0] as f32 / 10000.0),
                pre_fader[1] * (t.volume as f32 * balance[1] as f32 / 10000.0),
            ];

            sidechain_tap[idx] = if t.muted { [0.0; 2] } else { post_fader };

            if t.muted || (any_solo && !t.solo) {
                tapped_frames[idx] = [0; 2];
                gr_q8[idx] = 0;
                continue;
            }

            tapped_frames[idx] = parametric_eq::f32_to_frame(post_fader);
            gr_q8[idx] = dsp_math::round_f32(gr_db * 256.0).clamp(0.0, 32767.0) as i16;

            if let Some((dst, send_pct)) = t.post_fader_send {
                let di = dst as usize;
                if di < n && di != idx {
                    let s = send_pct as f32 / 100.0;
                    bus_accum[di][0] += post_fader[0] * s;
                    bus_accum[di][1] += post_fader[1] * s;
                }
            }

            if let Some(bus) = t.output_bus {
                let bi = bus as usize;
                if bi < n && bi != idx {
                    bus_accum[bi][0] += post_fader[0];
                    bus_accum[bi][1] += post_fader[1];
                } else {
                    master_mix[0] += post_fader[0];
                    master_mix[1] += post_fader[1];
                }
            } else {
                master_mix[0] += post_fader[0];
                master_mix[1] += post_fader[1];
            }
        }

        for idx in 0..n {
            tap(idx, tapped_frames[idx], gr_q8[idx]);
        }

        // Any extra tracks beyond MAX_CONSOLE_TRACKS sum directly to master without routing buffers.
        for (idx, t) in tracks.iter_mut().enumerate().skip(n) {
            let mut clip_sum = [0i64; 2];
            for clip in &t.clips {
                let cf = clip.sample(at);
                clip_sum[0] = clip_sum[0].saturating_add(cf[0]);
                clip_sum[1] = clip_sum[1].saturating_add(cf[1]);
            }
            let track_in = [clip_sum[0] as f32 / 32768.0, clip_sum[1] as f32 / 32768.0];
            let (pre_fader, gr_db) = t.process_console_frame(track_in, None);
            if t.muted || (any_solo && !t.solo) {
                tap(idx, [0; 2], 0);
                continue;
            }
            let balance = [100 - t.pan.max(0) as i64, 100 + t.pan.min(0) as i64];
            let post_fader = [
                pre_fader[0] * (t.volume as f32 * balance[0] as f32 / 10000.0),
                pre_fader[1] * (t.volume as f32 * balance[1] as f32 / 10000.0),
            ];
            let gr = dsp_math::round_f32(gr_db * 256.0).clamp(0.0, 32767.0) as i16;
            tap(idx, parametric_eq::f32_to_frame(post_fader), gr);
            master_mix[0] += post_fader[0];
            master_mix[1] += post_fader[1];
        }

        let mut master_out = [master_mix[0] * master_scale, master_mix[1] * master_scale];
        if let Some(m) = mastering.as_deref_mut() {
            master_out = m.process_stereo_f32(master_out);
        }
        *dest = parametric_eq::f32_to_frame(master_out);
    }

    let next_pos = start.checked_add(out.len());
    for t in tracks {
        t.processed_until = next_pos;
    }
    if let Some(m) = mastering {
        m.processed_until = next_pos;
    }
}

