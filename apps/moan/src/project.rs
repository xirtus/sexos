// SPDX-License-Identifier: GPL-3.0-only
//! Versioned editable Moan project, not a mixdown. PCM source IDs preserve shared
//! clip sources, edits and effect configuration. DSP history/undo are not stored.
use crate::{Session, MAX_FRAMES, MAX_TRACKS};
use alloc::{sync::Arc, vec::Vec};
use moan_core::{
    effects::{Compressor, Equalizer, Reverb},
    Clip, Frame, Track, RATE,
};
pub const MAGIC: &[u8; 8] = b"MOANPRJ\0";
pub const VERSION: u16 = 1;
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
    fn u16(&mut self, v: u16) -> Result<(), Error> {
        self.bytes(&v.to_le_bytes())
    }
    fn u32(&mut self, v: usize) -> Result<(), Error> {
        self.bytes(&u32::try_from(v).map_err(|_| Error::Limit)?.to_le_bytes())
    }
    fn u64(&mut self, v: usize) -> Result<(), Error> {
        self.bytes(&(v as u64).to_le_bytes())
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
    let mut sources: Vec<&[Frame]> = Vec::new();
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
            if !sources
                .iter()
                .any(|s| s.as_ptr() == source.as_ptr() && s.len() == source.len())
            {
                total = total.checked_add(source.len()).ok_or(Error::Limit)?;
                if total > MAX_PCM {
                    return Err(Error::Limit);
                }
                sources.push(source);
            }
        }
    }
    let mut w = Writer(Vec::new());
    w.bytes(MAGIC)?;
    w.u16(VERSION)?;
    w.u16(0)?;
    w.u32(RATE as usize)?;
    w.u64(0)?;
    w.u64(0)?;
    w.u16(sources.len() as u16)?;
    w.u16(session.tracks.len() as u16)?;
    w.u16(session.selected as u16)?;
    w.u16(0)?;
    for value in [session.cursor, session.from, session.to] {
        w.u64(value)?;
    }
    for source in &sources {
        w.u32(source.len())?;
        for frame in *source {
            for sample in frame {
                w.bytes(&sample.to_le_bytes())?;
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
                .position(|s| s.as_ptr() == source.as_ptr() && s.len() == source.len())
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
    if r.u16()? != VERSION {
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
    r.zero()?;
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
    if r.at != bytes.len() || used.iter().any(|used| !*used) {
        return Err(Error::Metadata);
    }
    // All structure is validated before PCM allocations or mutating any session.
    let mut audio: Vec<Arc<[Frame]>> = Vec::new();
    audio.try_reserve(sources).map_err(|_| Error::Memory)?;
    for bytes in source_bytes {
        let mut pcm = Vec::new();
        pcm.try_reserve_exact(bytes.len() / 4)
            .map_err(|_| Error::Memory)?;
        for b in bytes.chunks_exact(4) {
            pcm.push([
                i16::from_le_bytes([b[0], b[1]]),
                i16::from_le_bytes([b[2], b[3]]),
            ]);
        }
        audio.push(Arc::from(pcm));
    }
    let mut session = Session::default();
    session.selected = selected;
    session.cursor = cursor;
    session.from = from;
    session.to = to;
    for spec in specs {
        let mut track = Track::new();
        track.set_volume(spec.volume).map_err(|_| Error::Metadata)?;
        track.set_pan(spec.pan).map_err(|_| Error::Metadata)?;
        track.muted = spec.flags & 1 != 0;
        track.solo = spec.flags & 2 != 0;
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
            let mut clip = Clip::owned(audio[c.id].clone(), 0).map_err(|_| Error::Source)?;
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
    Ok(session)
}
