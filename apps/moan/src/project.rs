// SPDX-License-Identifier: GPL-3.0-only
//! Versioned editable Moan project, not a mixdown. PCM source IDs preserve shared
//! clip sources, edits and effect configuration. DSP history/undo are not stored.
use crate::{Session, MAX_FRAMES, MAX_TRACKS};
use alloc::{string::String, sync::Arc, vec::Vec};
use moan_core::{
    compose::ExpressiveNote,
    effects::{Compressor, Equalizer, Reverb},
    mastering::MasteringProcessor,
    parametric_eq::{BandConfig, ParametricEq},
    pattern::{
        ChannelId, PatternId, PatternInstance, QueuedScene, RackChannel, SceneMatrix,
        TapestryBank, TapestryPattern, TransportContext, MAX_INSTANCES, MAX_PATTERNS,
        MAX_RACK_CHANNELS, SCENE_COUNT, SCENE_LANES,
    },
    tape_fx::{
        BitCrusher, SidechainCompressor, SidechainGate, StereoChorus, TapeMachine,
        TransientShaper,
    },
    time::{TempoMap, TempoSegment, Time, TimeSignature, MAX_SWING_PCT},
    Clip, Track, RATE,
};
pub const MAGIC: &[u8; 8] = b"MOANPRJ\0";
pub const VERSION: u16 = 4;
pub const V3_VERSION: u16 = 3;
pub const MIDI_VERSION: u16 = 2;
pub const V4_VERSION: u16 = 4;
pub const HEADER: usize = 64;
pub const MAX_BYTES: usize = 96 * 1024 * 1024;
pub const MAX_CLIPS: usize = 1024;
pub const MAX_TIMELINE: usize = RATE as usize * 3660;
const MAX_PCM: usize = MAX_FRAMES * MAX_TRACKS;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Header,
    Version,
    Limit,
    Truncated,
    Checksum,
    Metadata,
    Source,
    Effect,
    Memory,
}
fn hash(bytes: &[u8]) -> u64 {
    bytes[..24]
        .iter()
        .chain(&bytes[32..])
        .fold(0xcbf29ce484222325, |v, b| {
            (v ^ *b as u64).wrapping_mul(0x100000001b3)
        })
}
struct Writer(Vec<u8>);
impl Writer {
    fn bytes(&mut self, b: &[u8]) -> Result<(), Error> {
        let n = self.0.len().checked_add(b.len()).ok_or(Error::Limit)?;
        if n > MAX_BYTES {
            return Err(Error::Limit);
        }
        self.0.try_reserve(b.len()).map_err(|_| Error::Memory)?;
        self.0.extend_from_slice(b);
        Ok(())
    }
    fn u8(&mut self, v: u8) -> Result<(), Error> {
        self.bytes(&[v])
    }
    fn i8(&mut self, v: i8) -> Result<(), Error> {
        self.bytes(&[v as u8])
    }
    fn u16(&mut self, v: u16) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn u32(&mut self, v: usize) -> Result<(), Error> {
        self.bytes(&u32::try_from(v).map_err(|_| Error::Limit)?.to_le_bytes())
    }
    fn u32_raw(&mut self, v: u32) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn i64(&mut self, v: i64) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn u64(&mut self, v: usize) -> Result<(), Error> {
        self.bytes(&(v as u64).to_le_bytes())
    }
    fn u64_raw(&mut self, v: u64) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn f32(&mut self, v: f32) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn str_u16(&mut self, s: &str) -> Result<(), Error> {
        let b = s.as_bytes();
        self.u16(u16::try_from(b.len()).map_err(|_| Error::Limit)?)?;
        self.bytes(b)
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or(Error::Limit)?;
        let out = self.bytes.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.bytes(1)?[0])
    }
    fn i8(&mut self) -> Result<i8, Error> {
        Ok(self.bytes(1)?[0] as i8)
    }
    fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<usize, Error> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()) as usize)
    }
    fn u32_raw(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<usize, Error> {
        usize::try_from(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
            .map_err(|_| Error::Limit)
    }
    fn u64_raw(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    fn str_u16(&mut self, max_len: usize) -> Result<String, Error> {
        let len = self.u16()? as usize;
        if len > max_len {
            return Err(Error::Limit);
        }
        let s = core::str::from_utf8(self.bytes(len)?).map_err(|_| Error::Metadata)?;
        Ok(String::from(s))
    }
    fn zero(&mut self) -> Result<(), Error> {
        if self.u16()? != 0 {
            return Err(Error::Metadata);
        }
        Ok(())
    }
}
pub fn encode(session: &Session) -> Result<Vec<u8>, Error> {
    if session.tracks.len() > MAX_TRACKS
        || session.selected >= session.tracks.len().max(1)
        || [session.cursor, session.from, session.to]
            .iter()
            .any(|v| *v > MAX_TIMELINE)
    {
        return Err(Error::Limit);
    }
    let mut sources: Vec<&Clip<'_>> = Vec::new();
    let mut clips = 0usize;
    let mut total = 0usize;
    for track in &session.tracks {
        for clip in &track.clips {
            clips += 1;
            if clips > MAX_CLIPS || clip.is_empty() || clip.end() > MAX_TIMELINE {
                return Err(Error::Limit);
            }
            let source = clip.source_frames();
            if source.is_empty() || source.len() > MAX_FRAMES {
                return Err(Error::Source);
            }
            if !sources.iter().any(|s| {
                s.source_frames().as_ptr() == source.as_ptr()
                    && s.source_frames().len() == source.len()
            }) {
                total = total.checked_add(source.len()).ok_or(Error::Limit)?;
                if total > MAX_PCM {
                    return Err(Error::Limit);
                }
                if let Some(pattern) = clip.midi_pattern() {
                    pattern.validate().map_err(|_| Error::Metadata)?;
                    if pattern.frames as usize != source.len() {
                        return Err(Error::Source);
                    }
                }
                sources.push(clip);
            }
        }
    }
    let mut w = Writer(Vec::new());
    w.bytes(MAGIC)?;
    let has_ext5 = session_needs_ext5(session, &sources);
    let has_v4 = has_ext5
        || sources.iter().any(|s| s.vocal_clip().is_some())
        || sources.iter().any(|s| {
            if let Some(p) = s.midi_pattern() {
                matches!(
                    p.instrument,
                    moan_core::midi::Instrument::ArielFormant
                        | moan_core::midi::Instrument::PrismWavetable
                )
            } else {
                false
            }
        })
        || session.tracks.iter().any(|t| t.distortion.is_some())
        || !session.rack.nodes.is_empty()
        || !session.muse_crate.is_empty();

    let version = if has_v4 {
        VERSION
    } else if session.master_volume != 100 {
        V3_VERSION
    } else if sources.iter().any(|source| source.midi_pattern().is_some()) {
        MIDI_VERSION
    } else {
        1
    };
    w.u16(version)?;
    w.u16(0)?;
    w.u32(RATE as usize)?;
    w.u64(0)?;
    w.u64(0)?;
    w.u16(sources.len() as u16)?;
    w.u16(session.tracks.len() as u16)?;
    w.u16(session.selected as u16)?;
    // v1/v2 reserved this word and retain their exact unity encoding.
    w.u16(if version >= 3 {
        session.master_volume
    } else {
        0
    })?;
    for value in [session.cursor, session.from, session.to] {
        w.u64(value)?;
    }
    for clip in &sources {
        let source = clip.source_frames();
        w.u32(source.len())?;
        for frame in source {
            for sample in frame {
                w.bytes(&sample.to_le_bytes())?;
            }
        }
        if version >= 2 {
            if let Some(pattern) = clip.midi_pattern() {
                w.u16(1)?;
                w.u16(pattern.instrument as u16)?;
                w.u16(pattern.bpm)?;
                w.u16(pattern.notes.len() as u16)?;
                for note in &pattern.notes {
                    w.u32(note.start as usize)?;
                    w.u32(note.frames as usize)?;
                    w.u16(note.key as u16)?;
                    w.u16(note.velocity as u16)?;
                }
            } else {
                w.u16(0)?;
                w.u16(0)?;
            }
        }
    }
    for t in &session.tracks {
        w.u16(t.clips.len() as u16)?;
        w.u16(t.volume())?;
        w.bytes(&t.pan().to_le_bytes())?;
        let fx = &t.effects;
        let flags = (fx.eq.is_some() as u8)
            | ((fx.compressor.is_some() as u8) << 1)
            | ((fx.reverb.is_some() as u8) << 2);
        w.bytes(&[(t.muted as u8) | ((t.solo as u8) << 1), flags])?;
        for gain in fx.eq.as_ref().map_or([0; 3], Equalizer::gains) {
            w.u16(gain)?;
        }
        let (threshold, ratio, attack, release) = fx
            .compressor
            .as_ref()
            .map_or((0, 0, 0, 0), Compressor::parameters);
        w.u16(threshold)?;
        w.u16(ratio)?;
        w.u32(attack as usize)?;
        w.u32(release as usize)?;
        let (feedback, wet) = fx.reverb.as_ref().map_or((0, 0), Reverb::parameters);
        w.u16(feedback)?;
        w.u16(wet)?;
        for clip in &t.clips {
            let source = clip.source_frames();
            let id = sources
                .iter()
                .position(|s| {
                    s.source_frames().as_ptr() == source.as_ptr()
                        && s.source_frames().len() == source.len()
                })
                .ok_or(Error::Source)?;
            w.u16(id as u16)?;
            w.u16(0)?;
            w.u64(clip.start())?;
            w.u32(clip.source_offset())?;
            w.u32(clip.len())?;
            w.u16(clip.gain())?;
            w.u16(0)?;
            let (incoming, outgoing) = clip.fades();
            w.u32(incoming)?;
            w.u32(outgoing)?;
        }
    }
    if version >= 4 {
        w.bytes(b"EXT4")?;
        // 1. Vocal sources metadata
        let vocal_sources: Vec<(usize, &moan_core::VocalClip)> = sources
            .iter()
            .enumerate()
            .filter_map(|(idx, s)| s.vocal_clip().map(|v| (idx, v)))
            .collect();
        w.u16(vocal_sources.len() as u16)?;
        for (idx, vocal) in vocal_sources {
            w.u16(idx as u16)?;
            let text_bytes = vocal.text.as_bytes();
            w.u16(text_bytes.len() as u16)?;
            w.bytes(text_bytes)?;
            w.f32(vocal.params.f0)?;
            w.f32(vocal.params.rate_ms)?;
            w.f32(vocal.params.formant_scale)?;
            w.f32(vocal.params.vibrato_depth)?;
            w.f32(vocal.params.vibrato_rate)?;
            w.f32(vocal.params.tremolo_depth)?;
            w.f32(vocal.params.tremolo_rate)?;
            w.f32(vocal.params.aspiration)?;
            w.f32(vocal.params.tilt)?;
            w.f32(vocal.params.effort)?;
            w.f32(vocal.params.gain)?;
            w.u16(vocal.spans.len() as u16)?;
            for span in &vocal.spans {
                w.u32(span.byte_start)?;
                w.u32(span.byte_end)?;
                w.u32(span.start_frame)?;
                w.u32(span.end_frame)?;
            }
        }
        // 2. Track distortions
        w.u16(session.tracks.len() as u16)?;
        for track in &session.tracks {
            if let Some(dist) = &track.distortion {
                w.u8(1)?;
                let (drive, tilt, bias, mix, cutoff, mode) = dist.parameters();
                w.u16(drive)?;
                w.bytes(&tilt.to_le_bytes())?;
                w.bytes(&bias.to_le_bytes())?;
                w.u16(mix)?;
                w.u16(cutoff)?;
                w.u8(mode as u8)?;
            } else {
                w.u8(0)?;
            }
        }
        // 3. Track automation lanes count
        w.u16(session.tracks.len() as u16)?;
        for _ in &session.tracks {
            w.u16(0)?;
        }
        // 4. EuropaRack PatchGraph
        if !session.rack.nodes.is_empty() {
            let patch_bytes = moan_modular::serialize::encode_patch(&session.rack)
                .map_err(|_| Error::Metadata)?;
            w.u32(patch_bytes.len())?;
            w.bytes(&patch_bytes)?;
        } else {
            w.u32(0)?;
        }
        // 5. Muse Crate takes / provenance
        w.u16(session.muse_crate.len() as u16)?;
        for take in &session.muse_crate {
            w.u64(take.id as usize)?;
            w.u16(take.title.len() as u16)?;
            w.bytes(take.title.as_bytes())?;
            w.u16(take.prompt.len() as u16)?;
            w.bytes(take.prompt.as_bytes())?;
            w.u16(take.lyrics.len() as u16)?;
            w.bytes(take.lyrics.as_bytes())?;
            w.u16(take.bpm)?;
            w.u16(take.key_scale.len() as u16)?;
            w.bytes(take.key_scale.as_bytes())?;
            w.u8(take.role as u8)?;
            w.u16(take.active_version as u16)?;
            w.u16(take.lrc.len() as u16)?;
            for line in &take.lrc {
                w.u32(line.timestamp_ms as usize)?;
                w.u32(line.frame_offset)?;
                w.u16(line.text.len() as u16)?;
                w.bytes(line.text.as_bytes())?;
            }
        }
        if has_ext5 {
            write_ext5(&mut w, session)?;
        }
    }
    let length = w.0.len();
    w.0[16..24].copy_from_slice(&(length as u64).to_le_bytes());
    let checksum = hash(&w.0);
    w.0[24..32].copy_from_slice(&checksum.to_le_bytes());
    Ok(w.0)
}
struct TrackSpec {
    volume: u16,
    pan: i16,
    flags: u8,
    fx: u8,
    eq: [u16; 3],
    compressor: (u16, u16, u32, u32),
    reverb: (u16, u16),
    clips: Vec<ClipSpec>,
}
struct ClipSpec {
    id: usize,
    start: usize,
    offset: usize,
    len: usize,
    gain: u16,
    incoming: usize,
    outgoing: usize,
}
pub fn decode(bytes: &[u8]) -> Result<Session, Error> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::Limit);
    }
    if bytes.len() < HEADER {
        return Err(Error::Truncated);
    }
    let mut r = Reader { bytes, at: 0 };
    if r.bytes(8)? != MAGIC {
        return Err(Error::Header);
    }
    let version = r.u16()?;
    if ![1, MIDI_VERSION, V3_VERSION, VERSION].contains(&version) {
        return Err(Error::Version);
    }
    r.zero()?;
    if r.u32()? != RATE as usize || r.u64()? != bytes.len() {
        return Err(Error::Header);
    }
    let checksum = u64::from_le_bytes(r.bytes(8)?.try_into().unwrap());
    if checksum != hash(bytes) {
        return Err(Error::Checksum);
    }
    let sources = r.u16()? as usize;
    let tracks = r.u16()? as usize;
    let selected = r.u16()? as usize;
    let master = r.u16()?;
    let master_volume = if version >= 3 {
        if master > 200 {
            return Err(Error::Metadata);
        }
        master
    } else {
        if master != 0 {
            return Err(Error::Metadata);
        }
        100
    };
    let cursor = r.u64()?;
    let from = r.u64()?;
    let to = r.u64()?;
    if sources > MAX_CLIPS
        || tracks > MAX_TRACKS
        || selected >= tracks.max(1)
        || [cursor, from, to].iter().any(|v| *v > MAX_TIMELINE)
    {
        return Err(Error::Limit);
    }
    let mut source_bytes = Vec::new();
    source_bytes
        .try_reserve(sources)
        .map_err(|_| Error::Memory)?;
    let mut source_patterns = Vec::new();
    source_patterns
        .try_reserve(sources)
        .map_err(|_| Error::Memory)?;
    let mut total = 0usize;
    for _ in 0..sources {
        let n = r.u32()?;
        if n == 0 || n > MAX_FRAMES {
            return Err(Error::Source);
        }
        total = total.checked_add(n).ok_or(Error::Limit)?;
        if total > MAX_PCM {
            return Err(Error::Limit);
        }
        source_bytes.push(r.bytes(n.checked_mul(4).ok_or(Error::Limit)?)?);
        let pattern = if version >= 2 {
            let tag = r.u16()?;
            let instrument = r.u16()?;
            match tag {
                0 => {
                    if instrument != 0 {
                        return Err(Error::Metadata);
                    }
                    None
                }
                1 => {
                    let instrument = moan_core::midi::Instrument::try_from(
                        u8::try_from(instrument).map_err(|_| Error::Metadata)?,
                    )
                    .map_err(|_| Error::Metadata)?;
                    let bpm = r.u16()?;
                    let count = r.u16()? as usize;
                    if count > moan_core::midi::MAX_NOTES {
                        return Err(Error::Limit);
                    }
                    let mut notes = Vec::new();
                    notes.try_reserve(count).map_err(|_| Error::Memory)?;
                    for _ in 0..count {
                        notes.push(moan_core::midi::Note {
                            start: r.u32()? as u32,
                            frames: r.u32()? as u32,
                            key: u8::try_from(r.u16()?).map_err(|_| Error::Metadata)?,
                            velocity: u8::try_from(r.u16()?).map_err(|_| Error::Metadata)?,
                        });
                    }
                    let pattern = moan_core::midi::Pattern {
                        frames: n as u32,
                        bpm,
                        instrument,
                        notes,
                    };
                    pattern.validate().map_err(|_| Error::Metadata)?;
                    Some(pattern)
                }
                _ => return Err(Error::Metadata),
            }
        } else {
            None
        };
        source_patterns.push(pattern);
    }
    let mut specs = Vec::new();
    specs.try_reserve(tracks).map_err(|_| Error::Memory)?;
    let mut count = 0usize;
    let mut used = alloc::vec![false;sources];
    for _ in 0..tracks {
        let n = r.u16()? as usize;
        count += n;
        if count > MAX_CLIPS {
            return Err(Error::Limit);
        }
        let volume = r.u16()?;
        let pan = r.u16()? as i16;
        let flags = r.bytes(1)?[0];
        let fx = r.bytes(1)?[0];
        if volume > 400 || !(-100..=100).contains(&pan) || flags & !3 != 0 || fx & !7 != 0 {
            return Err(Error::Metadata);
        }
        let eq = [r.u16()?, r.u16()?, r.u16()?];
        let compressor = (r.u16()?, r.u16()?, r.u32()? as u32, r.u32()? as u32);
        let reverb = (r.u16()?, r.u16()?);
        if (fx & 1 == 0 && eq != [0; 3])
            || (fx & 1 != 0 && eq.iter().any(|g| *g > 400))
            || (fx & 2 == 0 && compressor != (0, 0, 0, 0))
            || (fx & 2 != 0
                && (compressor.0 == 0
                    || compressor.0 > 32767
                    || !(1..=20).contains(&compressor.1)
                    || compressor.2 == 0
                    || compressor.3 == 0
                    || compressor.2 > 480000
                    || compressor.3 > 480000))
            || (fx & 4 == 0 && reverb != (0, 0))
            || (fx & 4 != 0 && (reverb.0 > 90 || reverb.1 > 100))
        {
            return Err(Error::Effect);
        }
        let mut clips = Vec::new();
        clips.try_reserve(n).map_err(|_| Error::Memory)?;
        for _ in 0..n {
            let id = r.u16()? as usize;
            r.zero()?;
            let start = r.u64()?;
            let offset = r.u32()?;
            let len = r.u32()?;
            let gain = r.u16()?;
            r.zero()?;
            let incoming = r.u32()?;
            let outgoing = r.u32()?;
            if id >= sources
                || len == 0
                || gain > 400
                || incoming > len
                || outgoing > len
                || start.checked_add(len).is_none_or(|end| end > MAX_TIMELINE)
                || offset
                    .checked_add(len)
                    .is_none_or(|end| end > source_bytes[id].len() / 4)
            {
                return Err(Error::Source);
            }
            used[id] = true;
            clips.push(ClipSpec {
                id,
                start,
                offset,
                len,
                gain,
                incoming,
                outgoing,
            });
        }
        specs.push(TrackSpec {
            volume,
            pan,
            flags,
            fx,
            eq,
            compressor,
            reverb,
            clips,
        });
    }
    if used.iter().any(|used| !*used) {
        return Err(Error::Metadata);
    }
    let mut vocal_metadata: Vec<(usize, moan_core::VocalClip)> = Vec::new();
    let mut track_distortions: Vec<Option<moan_core::effects::Distortion>> = Vec::new();
    let mut rack = moan_modular::PatchGraph::new();
    let mut muse_crate = Vec::new();

    if version >= 4 {
        if r.bytes(4)? != b"EXT4" {
            return Err(Error::Metadata);
        }
        // 1. Vocal sources metadata
        let vocal_count = r.u16()? as usize;
        for _ in 0..vocal_count {
            let source_idx = r.u16()? as usize;
            let text_len = r.u16()? as usize;
            let text = alloc::string::String::from(
                core::str::from_utf8(r.bytes(text_len)?).map_err(|_| Error::Metadata)?,
            );
            let params = moan_core::vocal::VoiceParams {
                f0: r.f32()?,
                rate_ms: r.f32()?,
                formant_scale: r.f32()?,
                vibrato_depth: r.f32()?,
                vibrato_rate: r.f32()?,
                tremolo_depth: r.f32()?,
                tremolo_rate: r.f32()?,
                aspiration: r.f32()?,
                tilt: r.f32()?,
                effort: r.f32()?,
                gain: r.f32()?,
            };
            let span_count = r.u16()? as usize;
            let mut spans = Vec::new();
            spans.try_reserve(span_count).map_err(|_| Error::Memory)?;
            for _ in 0..span_count {
                spans.push(moan_core::vocal::PhraseSpan {
                    byte_start: r.u32()?,
                    byte_end: r.u32()?,
                    start_frame: r.u32()?,
                    end_frame: r.u32()?,
                });
            }
            vocal_metadata.push((
                source_idx,
                moan_core::VocalClip {
                    text,
                    params,
                    spans,
                },
            ));
        }
        // 2. Track distortions
        let dist_count = r.u16()? as usize;
        for _ in 0..dist_count {
            let has = r.u8()?;
            if has != 0 {
                let drive = r.u16()?;
                let tilt = r.i16()?;
                let bias = r.i16()?;
                let mix = r.u16()?;
                let cutoff = r.u16()?;
                let mode_u8 = r.u8()?;
                let mode = match mode_u8 {
                    0 => moan_core::effects::DistortionMode::Symmetric,
                    1 => moan_core::effects::DistortionMode::Asymmetric,
                    2 => moan_core::effects::DistortionMode::HardClip,
                    _ => moan_core::effects::DistortionMode::Foldback,
                };
                let dist = moan_core::effects::Distortion::new(drive, tilt, bias, mix, cutoff, mode)
                    .map_err(|_| Error::Effect)?;
                track_distortions.push(Some(dist));
            } else {
                track_distortions.push(None);
            }
        }
        // 3. Track automation lanes count
        let auto_tracks = r.u16()? as usize;
        for _ in 0..auto_tracks {
            let _lanes = r.u16()?;
        }
        // 4. EuropaRack PatchGraph
        let rack_len = r.u32()?;
        if rack_len > 0 {
            let patch_bytes = r.bytes(rack_len)?;
            rack = moan_modular::serialize::decode_patch(patch_bytes)
                .map_err(|_| Error::Metadata)?;
        }
        // 5. Muse Crate takes / provenance
        let takes_count = r.u16()? as usize;
        for _ in 0..takes_count {
            let id = r.u64()? as u64;
            let title_len = r.u16()? as usize;
            let title = alloc::string::String::from(
                core::str::from_utf8(r.bytes(title_len)?).map_err(|_| Error::Metadata)?,
            );
            let prompt_len = r.u16()? as usize;
            let prompt = alloc::string::String::from(
                core::str::from_utf8(r.bytes(prompt_len)?).map_err(|_| Error::Metadata)?,
            );
            let lyrics_len = r.u16()? as usize;
            let lyrics = alloc::string::String::from(
                core::str::from_utf8(r.bytes(lyrics_len)?).map_err(|_| Error::Metadata)?,
            );
            let bpm = r.u16()?;
            let key_len = r.u16()? as usize;
            let key_scale = alloc::string::String::from(
                core::str::from_utf8(r.bytes(key_len)?).map_err(|_| Error::Metadata)?,
            );
            let role_u8 = r.u8()?;
            let role = match role_u8 {
                0 => moan_gen::job::StemRole::Vocals,
                1 => moan_gen::job::StemRole::Drums,
                2 => moan_gen::job::StemRole::Bass,
                3 => moan_gen::job::StemRole::Chords,
                4 => moan_gen::job::StemRole::Fx,
                _ => moan_gen::job::StemRole::Other,
            };
            let active_version = r.u16()? as usize;
            let lrc_len = r.u16()? as usize;
            let mut lrc = Vec::new();
            for _ in 0..lrc_len {
                let timestamp_ms = r.u32()? as u32;
                let frame_offset = r.u32()?;
                let line_len = r.u16()? as usize;
                let line_text = alloc::string::String::from(
                    core::str::from_utf8(r.bytes(line_len)?).map_err(|_| Error::Metadata)?,
                );
                lrc.push(moan_gen::lrc::LrcLine {
                    timestamp_ms,
                    frame_offset,
                    text: line_text,
                });
            }
            let mut take = moan_gen::stems::TakeCrate::new(id, &title, &prompt, &lyrics, bpm, &key_scale, role);
            take.active_version = active_version;
            take.lrc = lrc;
            muse_crate.push(take);
        }
    }
    let mut ext5_data: Option<Ext5Data> = None;
    if version >= 4 && r.at < bytes.len() {
        if r.bytes(4)? != b"EXT5" {
            return Err(Error::Metadata);
        }
        ext5_data = Some(read_ext5(&mut r, specs.len())?);
    }
    if r.at != bytes.len() {
        return Err(Error::Metadata);
    }
    // All structure is validated before PCM allocations or mutating any session.
    let mut audio: Vec<Clip<'static>> = Vec::new();
    audio.try_reserve(sources).map_err(|_| Error::Memory)?;
    for (src_idx, (bytes, pattern)) in source_bytes.into_iter().zip(source_patterns).enumerate() {
        if let Some(pattern) = pattern {
            let clip = Clip::midi(pattern, 0).map_err(|_| Error::Memory)?;
            if clip
                .source_frames()
                .iter()
                .zip(bytes.chunks_exact(4))
                .any(|(f, b)| {
                    f[0] != i16::from_le_bytes([b[0], b[1]])
                        || f[1] != i16::from_le_bytes([b[2], b[3]])
                })
            {
                return Err(Error::Source);
            }
            audio.push(clip);
            continue;
        }
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(bytes.len() / 4)
            .map_err(|_| Error::Memory)?;
        for b in bytes.chunks_exact(4) {
            pcm.push([
                i16::from_le_bytes([b[0], b[1]]),
                i16::from_le_bytes([b[2], b[3]]),
            ]);
        }
        let pcm_arc = Arc::new(pcm);
        if let Some((_, vocal)) = vocal_metadata.iter().find(|(idx, _)| *idx == src_idx) {
            audio.push(Clip::vocal_with_pcm(vocal.clone(), pcm_arc, 0).map_err(|_| Error::Source)?);
        } else {
            audio.push(Clip::buffer(pcm_arc, 0).map_err(|_| Error::Source)?);
        }
    }
    let mut session = Session::default();
    session.selected = selected;
    session.master_volume = master_volume;
    session.cursor = cursor;
    session.from = from;
    session.to = to;
    for (t_idx, spec) in specs.into_iter().enumerate() {
        let mut track = Track::new();
        track.set_volume(spec.volume).map_err(|_| Error::Metadata)?;
        track.set_pan(spec.pan).map_err(|_| Error::Metadata)?;
        track.muted = spec.flags & 1 != 0;
        track.solo = spec.flags & 2 != 0;
        if let Some(dist) = track_distortions.get(t_idx).and_then(|d| d.clone()) {
            track.distortion = Some(dist);
        }
        if spec.fx & 1 != 0 {
            track.effects.eq = Some(
                Equalizer::new(spec.eq[0], spec.eq[1], spec.eq[2]).map_err(|_| Error::Effect)?,
            );
        }
        if spec.fx & 2 != 0 {
            let (a, b, c, d) = spec.compressor;
            track.effects.compressor =
                Some(Compressor::new(a, b, c, d).map_err(|_| Error::Effect)?);
        }
        if spec.fx & 4 != 0 {
            track.effects.reverb =
                Some(Reverb::new(spec.reverb.0, spec.reverb.1).map_err(|_| Error::Effect)?);
        }
        for c in spec.clips {
            let mut clip = audio[c.id].clone();
            clip.trim(c.offset, c.offset + c.len)
                .map_err(|_| Error::Source)?;
            clip.move_to(c.start).map_err(|_| Error::Source)?;
            clip.set_gain(c.gain).map_err(|_| Error::Source)?;
            clip.set_fades(c.incoming, c.outgoing)
                .map_err(|_| Error::Source)?;
            track.clips.push(clip);
        }
        session.tracks.push(track);
    }
    session.rack = rack;
    session.muse_crate = muse_crate;
    if let Some(ext5) = ext5_data {
        for (track, fx) in session.tracks.iter_mut().zip(ext5.track_fx.into_iter()) {
            track.parametric_eq = fx.parametric_eq;
            track.sc_compressor = fx.sc_compressor;
            track.gate = fx.gate;
            track.tape = fx.tape;
            track.transient = fx.transient;
            track.crusher = fx.crusher;
            track.chorus = fx.chorus;
            track.sidechain_from = fx.sidechain_from;
            track.post_fader_send = fx.post_fader_send;
            track.output_bus = fx.output_bus;
        }
        session.mastering = ext5.mastering;
        session.tapestry = ext5.tapestry;
        session.scenes = ext5.scenes;
    }
    Ok(session)
}

fn track_has_ext5(t: &Track<'_>) -> bool {
    t.parametric_eq.is_some()
        || t.sc_compressor.is_some()
        || t.gate.is_some()
        || t.tape.is_some()
        || t.transient.is_some()
        || t.crusher.is_some()
        || t.chorus.is_some()
        || t.sidechain_from.is_some()
        || t.post_fader_send.is_some()
        || t.output_bus.is_some()
}

fn scenes_has_data(scenes: &SceneMatrix) -> bool {
    scenes.active_scene.is_some()
        || scenes.queued.is_some()
        || scenes
            .scenes
            .iter()
            .any(|row| row.slots.iter().any(Option::is_some))
}

fn tapestry_has_data(bank: &TapestryBank) -> bool {
    !bank.channels.is_empty()
        || !bank.patterns.is_empty()
        || !bank.instances.is_empty()
        || bank.swing_pct != 0
        || bank.transport_mode != TransportContext::PatternMode
        || bank.tempo_map != TempoMap::default()
}

fn session_needs_ext5(session: &Session, sources: &[&Clip<'_>]) -> bool {
    tapestry_has_data(&session.tapestry)
        || scenes_has_data(&session.scenes)
        || session.mastering.is_some()
        || session.tracks.iter().any(track_has_ext5)
        || sources
            .iter()
            .any(|s| s.midi_pattern().is_some_and(|p| (p.instrument as u8) >= 5))
}

struct Ext5TrackFx {
    parametric_eq: Option<ParametricEq>,
    sc_compressor: Option<SidechainCompressor>,
    gate: Option<SidechainGate>,
    tape: Option<TapeMachine>,
    transient: Option<TransientShaper>,
    crusher: Option<BitCrusher>,
    chorus: Option<StereoChorus>,
    sidechain_from: Option<u8>,
    post_fader_send: Option<(u8, u16)>,
    output_bus: Option<u8>,
}

struct Ext5Data {
    track_fx: Vec<Ext5TrackFx>,
    mastering: Option<MasteringProcessor>,
    tapestry: TapestryBank,
    scenes: SceneMatrix,
}

fn write_ext5(w: &mut Writer, session: &Session) -> Result<(), Error> {
    w.bytes(b"EXT5")?;
    w.u16(session.tracks.len() as u16)?;
    for t in &session.tracks {
        let flags = (t.parametric_eq.is_some() as u16)
            | ((t.sc_compressor.is_some() as u16) << 1)
            | ((t.gate.is_some() as u16) << 2)
            | ((t.tape.is_some() as u16) << 3)
            | ((t.transient.is_some() as u16) << 4)
            | ((t.crusher.is_some() as u16) << 5)
            | ((t.chorus.is_some() as u16) << 6)
            | ((t.sidechain_from.is_some() as u16) << 7)
            | ((t.post_fader_send.is_some() as u16) << 8)
            | ((t.output_bus.is_some() as u16) << 9);
        w.u16(flags)?;
        if let Some(eq) = &t.parametric_eq {
            for b_idx in 0..6 {
                let b = eq.band(b_idx).ok_or(Error::Effect)?;
                w.u8(b.enabled as u8)?;
                w.f32(b.freq_hz)?;
                w.f32(b.gain_db)?;
                w.f32(b.q)?;
            }
        }
        if let Some(comp) = &t.sc_compressor {
            let (th, ra, at, re, mk) = comp.parameters();
            for v in [th, ra, at, re, mk] {
                w.f32(v)?;
            }
        }
        if let Some(gate) = &t.gate {
            let (th, hy, ho, at, re, rg) = gate.parameters();
            for v in [th, hy, ho, at, re, rg] {
                w.f32(v)?;
            }
        }
        if let Some(tape) = &t.tape {
            let (dr, hb, tn, wo, fl, hs, mx) = tape.parameters();
            for v in [dr, hb, tn, wo, fl, hs, mx] {
                w.f32(v)?;
            }
        }
        if let Some(tr) = &t.transient {
            let (at, su) = tr.parameters();
            w.f32(at)?;
            w.f32(su)?;
        }
        if let Some(cr) = &t.crusher {
            let (ds, bits, mix) = cr.parameters();
            w.u16(ds)?;
            w.u8(bits)?;
            w.f32(mix)?;
        }
        if let Some(ch) = &t.chorus {
            let (rt, dp, fb, mx) = ch.parameters();
            for v in [rt, dp, fb, mx] {
                w.f32(v)?;
            }
        }
        if let Some(sc) = t.sidechain_from {
            w.u8(sc)?;
        }
        if let Some((dst, pct)) = t.post_fader_send {
            w.u8(dst)?;
            w.u16(pct)?;
        }
        if let Some(bus) = t.output_bus {
            w.u8(bus)?;
        }
    }

    if let Some(m) = &session.mastering {
        w.u8(1)?;
        w.f32(m.trim_db())?;
        let (low, body, air) = m.eq_gains();
        w.f32(low)?;
        w.f32(body)?;
        w.f32(air)?;
        let (th, ra, at, re, mk) = m.glue_compressor().parameters();
        for v in [th, ra, at, re, mk] {
            w.f32(v)?;
        }
        w.f32(m.stereo_width())?;
        w.f32(m.limiter().ceiling_db())?;
        w.f32(m.limiter().release_ms())?;
    } else {
        w.u8(0)?;
    }

    let bank = &session.tapestry;
    w.u16(bank.active_pattern.0)?;
    w.u8(match bank.transport_mode {
        TransportContext::PatternMode => 0,
        TransportContext::SongMode => 1,
    })?;
    w.u8(bank.swing_pct)?;

    let segs = bank.tempo_map.segments();
    w.u16(segs.len() as u16)?;
    for s in segs {
        w.i64(s.start.num)?;
        w.u32_raw(s.start.den)?;
        w.u32_raw(s.bpm_x100)?;
        if let Some(r) = s.ramp_to_bpm_x100 {
            w.u8(1)?;
            w.u32_raw(r)?;
        } else {
            w.u8(0)?;
        }
    }
    let sigs = bank.tempo_map.time_signatures();
    w.u16(sigs.len() as u16)?;
    for &(at, sig) in sigs {
        w.i64(at.num)?;
        w.u32_raw(at.den)?;
        w.u8(sig.numerator)?;
        w.u8(sig.denominator)?;
    }

    w.u16(bank.channels.len() as u16)?;
    for ch in &bank.channels {
        w.u16(ch.id.0)?;
        w.str_u16(&ch.name)?;
        w.u8(ch.instrument)?;
        w.u8(ch.root_key)?;
        w.u16(ch.volume_pct)?;
        w.i8(ch.pan)?;
        w.u8((ch.muted as u8) | ((ch.solo as u8) << 1))?;
        w.u8(ch.cut_group)?;
        w.u8(ch.target_track)?;
    }

    w.u16(bank.patterns.len() as u16)?;
    for pat in &bank.patterns {
        w.u16(pat.id.0)?;
        w.str_u16(&pat.name)?;
        w.u32_raw(pat.color_rgb)?;
        w.u8(pat.bars)?;
        w.u8(pat.steps_per_beat)?;
        w.u16(pat.channel_notes.len() as u16)?;
        for (ch_id, notes) in &pat.channel_notes {
            w.u16(ch_id.0)?;
            w.u32(notes.len())?;
            for n in notes {
                w.u32_raw(n.id)?;
                w.i64(n.start.num)?;
                w.u32_raw(n.start.den)?;
                w.i64(n.length.num)?;
                w.u32_raw(n.length.den)?;
                w.u8(n.key)?;
                w.u8(n.velocity)?;
                w.i8(n.pan)?;
                w.i8(n.fine_cents)?;
                if let Some(slide) = n.slide_to_key {
                    w.u8(1)?;
                    w.u8(slide)?;
                } else {
                    w.u8(0)?;
                }
                w.u8(n.timbre)?;
                w.u8(n.pressure)?;
            }
        }
    }

    w.u16(bank.instances.len() as u16)?;
    for inst in &bank.instances {
        w.u16(inst.pattern.0)?;
        w.i64(inst.start_beat.num)?;
        w.u32_raw(inst.start_beat.den)?;
        w.i64(inst.length_beats.num)?;
        w.u32_raw(inst.length_beats.den)?;
        w.i64(inst.offset_beats.num)?;
        w.u32_raw(inst.offset_beats.den)?;
        w.u8(inst.lane)?;
    }

    let scenes = &session.scenes;
    w.u8(scenes.active_scene.map_or(255, |i| i as u8))?;
    if let Some(q) = scenes.queued {
        w.u8(1)?;
        w.u16(q.scene_idx as u16)?;
        w.i64(q.launch_beat.num)?;
        w.u32_raw(q.launch_beat.den)?;
        w.u64_raw(q.launch_frame)?;
    } else {
        w.u8(0)?;
    }
    w.u8(scenes.scenes.len() as u8)?;
    for row in &scenes.scenes {
        w.str_u16(&row.name)?;
        for slot in &row.slots {
            if let Some(pid) = slot {
                w.u8(1)?;
                w.u16(pid.0)?;
            } else {
                w.u8(0)?;
            }
        }
    }
    Ok(())
}

fn read_ext5(r: &mut Reader<'_>, expected_tracks: usize) -> Result<Ext5Data, Error> {
    let track_count = r.u16()? as usize;
    if track_count != expected_tracks || track_count > MAX_TRACKS {
        return Err(Error::Metadata);
    }
    let mut track_fx = Vec::new();
    track_fx.try_reserve(track_count).map_err(|_| Error::Memory)?;
    for _ in 0..track_count {
        let flags = r.u16()?;
        if (flags & !0x03FF) != 0 {
            return Err(Error::Metadata);
        }
        let parametric_eq = if flags & (1 << 0) != 0 {
            let mut eq = ParametricEq::new();
            for b_idx in 0..6 {
                let enabled = r.u8()? != 0;
                let freq_hz = r.f32()?;
                let gain_db = r.f32()?;
                let q = r.f32()?;
                eq.set_band(
                    b_idx,
                    BandConfig {
                        enabled,
                        freq_hz,
                        gain_db,
                        q,
                    },
                )
                .map_err(|_| Error::Effect)?;
            }
            Some(eq)
        } else {
            None
        };
        let sc_compressor = if flags & (1 << 1) != 0 {
            Some(
                SidechainCompressor::new(r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?)
                    .map_err(|_| Error::Effect)?,
            )
        } else {
            None
        };
        let gate = if flags & (1 << 2) != 0 {
            Some(
                SidechainGate::new(
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                )
                .map_err(|_| Error::Effect)?,
            )
        } else {
            None
        };
        let tape = if flags & (1 << 3) != 0 {
            Some(
                TapeMachine::new(
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                    r.f32()?,
                )
                .map_err(|_| Error::Effect)?,
            )
        } else {
            None
        };
        let transient = if flags & (1 << 4) != 0 {
            Some(TransientShaper::new(r.f32()?, r.f32()?).map_err(|_| Error::Effect)?)
        } else {
            None
        };
        let crusher = if flags & (1 << 5) != 0 {
            Some(BitCrusher::new(r.u16()?, r.u8()?, r.f32()?).map_err(|_| Error::Effect)?)
        } else {
            None
        };
        let chorus = if flags & (1 << 6) != 0 {
            Some(
                StereoChorus::new(r.f32()?, r.f32()?, r.f32()?, r.f32()?)
                    .map_err(|_| Error::Effect)?,
            )
        } else {
            None
        };
        let sidechain_from = if flags & (1 << 7) != 0 {
            let k = r.u8()?;
            if (k as usize) >= MAX_TRACKS {
                return Err(Error::Metadata);
            }
            Some(k)
        } else {
            None
        };
        let post_fader_send = if flags & (1 << 8) != 0 {
            let dst = r.u8()?;
            let pct = r.u16()?;
            if (dst as usize) >= MAX_TRACKS || pct > 400 {
                return Err(Error::Metadata);
            }
            Some((dst, pct))
        } else {
            None
        };
        let output_bus = if flags & (1 << 9) != 0 {
            let bus = r.u8()?;
            if (bus as usize) >= MAX_TRACKS {
                return Err(Error::Metadata);
            }
            Some(bus)
        } else {
            None
        };
        track_fx.push(Ext5TrackFx {
            parametric_eq,
            sc_compressor,
            gate,
            tape,
            transient,
            crusher,
            chorus,
            sidechain_from,
            post_fader_send,
            output_bus,
        });
    }

    let mastering = if r.u8()? != 0 {
        let mut m = MasteringProcessor::new().map_err(|_| Error::Effect)?;
        m.set_trim_db(r.f32()?).map_err(|_| Error::Effect)?;
        m.set_eq_gains(r.f32()?, r.f32()?, r.f32()?)
            .map_err(|_| Error::Effect)?;
        m.set_glue_compressor(r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?)
            .map_err(|_| Error::Effect)?;
        m.set_stereo_width(r.f32()?).map_err(|_| Error::Effect)?;
        m.limiter_mut()
            .set_ceiling_db(r.f32()?)
            .map_err(|_| Error::Effect)?;
        m.limiter_mut()
            .set_release_ms(r.f32()?)
            .map_err(|_| Error::Effect)?;
        Some(m)
    } else {
        None
    };

    let mut bank = TapestryBank::new();
    let active_pattern = PatternId(r.u16()?);
    bank.transport_mode = match r.u8()? {
        0 => TransportContext::PatternMode,
        1 => TransportContext::SongMode,
        _ => return Err(Error::Metadata),
    };
    let swing_pct = r.u8()?;
    if swing_pct > MAX_SWING_PCT {
        return Err(Error::Metadata);
    }
    bank.swing_pct = swing_pct;

    let seg_count = r.u16()? as usize;
    if seg_count > 4096 {
        return Err(Error::Limit);
    }
    let mut segs = Vec::new();
    segs.try_reserve(seg_count).map_err(|_| Error::Memory)?;
    for _ in 0..seg_count {
        let start = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
        let bpm_x100 = r.u32_raw()?;
        let ramp_to_bpm_x100 = if r.u8()? != 0 {
            Some(r.u32_raw()?)
        } else {
            None
        };
        segs.push(TempoSegment {
            start,
            bpm_x100,
            ramp_to_bpm_x100,
        });
    }
    let sig_count = r.u16()? as usize;
    if sig_count > 1024 {
        return Err(Error::Limit);
    }
    let mut sigs = Vec::new();
    sigs.try_reserve(sig_count).map_err(|_| Error::Memory)?;
    for _ in 0..sig_count {
        let at = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
        let num = r.u8()?;
        let den = r.u8()?;
        let sig = TimeSignature::new(num, den).map_err(|_| Error::Metadata)?;
        sigs.push((at, sig));
    }
    bank.tempo_map = TempoMap::with_time_signatures(segs, sigs).map_err(|_| Error::Metadata)?;

    let ch_count = r.u16()? as usize;
    if ch_count > MAX_RACK_CHANNELS {
        return Err(Error::Limit);
    }
    for _ in 0..ch_count {
        let id = ChannelId(r.u16()?);
        let name = r.str_u16(256)?;
        let instrument = r.u8()?;
        let root_key = r.u8()?;
        let volume_pct = r.u16()?;
        let pan = r.i8()?;
        let flags = r.u8()?;
        let cut_group = r.u8()?;
        let target_track = r.u8()?;
        let mut ch = RackChannel::new(id, name, instrument);
        ch.root_key = root_key;
        ch.volume_pct = volume_pct;
        ch.pan = pan;
        ch.muted = flags & 1 != 0;
        ch.solo = flags & 2 != 0;
        ch.cut_group = cut_group;
        ch.target_track = target_track;
        bank.push_channel(ch).map_err(|_| Error::Metadata)?;
    }

    let pat_count = r.u16()? as usize;
    if pat_count > MAX_PATTERNS {
        return Err(Error::Limit);
    }
    for _ in 0..pat_count {
        let id = PatternId(r.u16()?);
        let name = r.str_u16(256)?;
        let color_rgb = r.u32_raw()?;
        let bars = r.u8()?;
        let steps_per_beat = r.u8()?;
        let mut pat = TapestryPattern::new(id, name, bars, steps_per_beat)
            .map_err(|_| Error::Metadata)?;
        pat.color_rgb = color_rgb;
        let ch_note_vecs = r.u16()? as usize;
        if ch_note_vecs > MAX_RACK_CHANNELS {
            return Err(Error::Limit);
        }
        for _ in 0..ch_note_vecs {
            let ch_id = ChannelId(r.u16()?);
            let note_count = r.u32()?;
            if note_count > 16_384 {
                return Err(Error::Limit);
            }
            for _ in 0..note_count {
                let n_id = r.u32_raw()?;
                let start = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
                let length = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
                let key = r.u8()?;
                let velocity = r.u8()?;
                let pan = r.i8()?;
                let fine_cents = r.i8()?;
                let slide_to_key = if r.u8()? != 0 { Some(r.u8()?) } else { None };
                let timbre = r.u8()?;
                let pressure = r.u8()?;
                let mut note = ExpressiveNote::new(n_id, start, length, key, velocity);
                note.pan = pan;
                note.fine_cents = fine_cents;
                note.slide_to_key = slide_to_key;
                note.timbre = timbre;
                note.pressure = pressure;
                pat.add_note(ch_id, note).map_err(|_| Error::Metadata)?;
            }
        }
        bank.push_pattern(pat).map_err(|_| Error::Metadata)?;
    }
    bank.active_pattern = active_pattern;

    let inst_count = r.u16()? as usize;
    if inst_count > MAX_INSTANCES {
        return Err(Error::Limit);
    }
    for _ in 0..inst_count {
        let pattern = PatternId(r.u16()?);
        let start_beat = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
        let length_beats = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
        let offset_beats = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
        let lane = r.u8()?;
        bank.place_instance(PatternInstance {
            pattern,
            start_beat,
            length_beats,
            offset_beats,
            lane,
        })
        .map_err(|_| Error::Metadata)?;
    }

    let mut scenes = SceneMatrix::new();
    let active_scene_u8 = r.u8()?;
    scenes.active_scene = if active_scene_u8 == 255 {
        None
    } else if (active_scene_u8 as usize) < SCENE_COUNT {
        Some(active_scene_u8 as usize)
    } else {
        return Err(Error::Metadata);
    };
    scenes.queued = if r.u8()? != 0 {
        let scene_idx = r.u16()? as usize;
        if scene_idx >= SCENE_COUNT {
            return Err(Error::Metadata);
        }
        let launch_beat = Time::new(r.i64()?, r.u32_raw()?).ok_or(Error::Metadata)?;
        let launch_frame = r.u64_raw()?;
        Some(QueuedScene {
            scene_idx,
            launch_beat,
            launch_frame,
        })
    } else {
        None
    };
    let scene_rows = r.u8()? as usize;
    if scene_rows > SCENE_COUNT {
        return Err(Error::Limit);
    }
    for s_idx in 0..scene_rows {
        let name = r.str_u16(256)?;
        if let Some(row) = scenes.scenes.get_mut(s_idx) {
            row.name = name;
        }
        for lane in 0..SCENE_LANES {
            let pat_opt = if r.u8()? != 0 {
                Some(PatternId(r.u16()?))
            } else {
                None
            };
            scenes
                .set_slot(s_idx, lane, pat_opt)
                .map_err(|_| Error::Metadata)?;
        }
    }

    Ok(Ext5Data {
        track_fx,
        mastering,
        tapestry: bank,
        scenes,
    })
}
