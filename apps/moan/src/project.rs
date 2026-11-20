// SPDX-License-Identifier: GPL-3.0-only
//! Versioned editable Moan project, not a mixdown. PCM source IDs preserve shared
//! clip sources, edits and effect configuration. DSP history/undo are not stored.
use crate::{voice, Session, MAX_FRAMES, MAX_TRACKS};
use alloc::{sync::Arc, vec::Vec};
use moan_core::{
    effects::{Compressor, Equalizer, Reverb},
    Clip, Track, RATE,
};
pub const MAGIC: &[u8; 8] = b"MOANPRJ\0";
/// v5: v4 plus per-track voice generation provenance.
pub const VERSION: u16 = 5;
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
    fn u16(&mut self, v: u16) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn u32(&mut self, v: usize) -> Result<(), Error> {
        self.bytes(&u32::try_from(v).map_err(|_| Error::Limit)?.to_le_bytes())
    }
    fn u64(&mut self, v: usize) -> Result<(), Error> {
        self.bytes(&(v as u64).to_le_bytes())
    }
    fn f32(&mut self, v: f32) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
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
    fn i16(&mut self) -> Result<i16, Error> {
        Ok(i16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<usize, Error> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()) as usize)
    }
    fn u64(&mut self) -> Result<usize, Error> {
        usize::try_from(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
            .map_err(|_| Error::Limit)
    }
    fn f32(&mut self) -> Result<f32, Error> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
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
    let has_v4 = sources.iter().any(|s| s.vocal_clip().is_some())
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

    let version = if session.generated.iter().any(Option::is_some) {
        VERSION
    } else if has_v4 {
        V4_VERSION
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
    w.u16(if version >= V3_VERSION {
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
    }
    if version >= VERSION {
        for index in 0..session.tracks.len() {
            match session.generated.get(index).and_then(Option::as_ref) {
                None => w.bytes(&[0])?,
                Some(p) => {
                    if p.job_id == 0 || p.asset_id == 0 || p.model_len == 0
                        || p.model_len as usize > voice::MODEL_MAX || !(1..=2).contains(&p.source_channels)
                        || !(8_000..=192_000).contains(&p.source_rate)
                        || core::str::from_utf8(&p.model[..p.model_len as usize]).is_err() { return Err(Error::Metadata); }
                    w.bytes(&[1])?;
                    for value in [p.job_id, p.asset_id, p.seed] { w.bytes(&value.to_le_bytes())?; }
                    w.bytes(&p.content_hash)?;
                    w.bytes(&p.source_rate.to_le_bytes())?;
                    w.bytes(&[p.source_channels, p.model_len])?;
                    w.bytes(&p.model[..p.model_len as usize])?;
                }
            }
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
    if ![1, MIDI_VERSION, V3_VERSION, V4_VERSION, VERSION].contains(&version) {
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
    let master_volume = if version >= V3_VERSION {
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
    let mut generated = Vec::new();
    generated.try_reserve(tracks).map_err(|_| Error::Memory)?;
    if version >= VERSION {
        for _ in 0..tracks {
            match r.bytes(1)?[0] {
                0 => generated.push(None),
                1 => {
                    let job_id = u64::from_le_bytes(r.bytes(8)?.try_into().unwrap());
                    let asset_id = u64::from_le_bytes(r.bytes(8)?.try_into().unwrap());
                    let seed = u64::from_le_bytes(r.bytes(8)?.try_into().unwrap());
                    let content_hash = r.bytes(32)?.try_into().unwrap();
                    let source_rate = r.u32()? as u32;
                    let source_channels = r.bytes(1)?[0];
                    let model_len = r.bytes(1)?[0];
                    if job_id == 0 || asset_id == 0 || model_len == 0
                        || model_len as usize > voice::MODEL_MAX
                        || !(8_000..=192_000).contains(&source_rate)
                        || !(1..=2).contains(&source_channels) { return Err(Error::Metadata); }
                    let model_bytes = r.bytes(model_len as usize)?;
                    core::str::from_utf8(model_bytes).map_err(|_| Error::Metadata)?;
                    let mut model = [0; voice::MODEL_MAX];
                    model[..model_len as usize].copy_from_slice(model_bytes);
                    generated.push(Some(voice::Provenance { job_id, asset_id, seed, content_hash,
                        source_rate, source_channels, model, model_len }));
                }
                _ => return Err(Error::Metadata),
            }
        }
    } else { generated.resize(tracks, None); }
    if r.at != bytes.len() || used.iter().any(|used| !*used) {
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
    session.generated = generated;
    Ok(session)
}
