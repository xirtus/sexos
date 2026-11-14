//! Export: walk the timeline frame by frame and ask for the right picture, and lay out
//! every piece of sound in samples. The codec side (sex-media's decoders and encoders)
//! sits behind [`Pictures`] and [`Frames`], so this logic is tested on the host with
//! synthetic sources and the output format can change without touching it.

use crate::render::{plan, Piece};
use crate::{Micros, Range, Timeline, TrackKind, SECOND};
use alloc::vec::Vec;

/// Gives the picture a source shows at a source time. An implementation keeps one
/// decoder per source and seeks only when asked for a time behind it or far ahead.
pub trait Pictures {
    type Error;
    fn picture(&mut self, media: u32, source: Micros, out: &mut [u32]) -> Result<(), Self::Error>;
}

/// Takes finished frames in order.
pub trait Frames {
    type Error;
    fn frame(&mut self, pixels: &[u32], at: Micros) -> Result<(), Self::Error>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExportError<P, F> {
    Picture(P),
    Output(F),
    /// The range is empty or lies outside the timeline, or the frame rate is zero.
    Empty,
}

/// The timeline time of output frame `n`, rounded up to the microsecond so that it never
/// falls before the frame's true start (at 30 fps frame 149 is 4 966 667, not 4 966 666,
/// which would still be inside source frame 148).
pub fn frame_time(tl: &Timeline, n: u64) -> Micros {
    let num = n as i128 * tl.fps_den as i128 * SECOND as i128;
    let d = tl.fps_num as i128;
    ((num + d - 1) / d) as Micros
}

/// What picture frame `n` shows: the topmost enabled picture clip there (later picture
/// tracks draw over earlier ones), as `(media, source time)`, or `None` for black.
pub fn picture_at(tl: &Timeline, t: Micros) -> Option<(u32, Micros)> {
    tl.tracks.iter().rev().filter(|tr| tr.kind == TrackKind::Video).find_map(|tr| {
        tr.clips.iter().find(|c| c.enabled && c.at <= t && t < c.end()).map(|c| (c.media, c.source_at(t)))
    })
}

/// Export the pictures of `range` (the whole timeline when `None`) at the timeline's
/// frame rate, `pixels` per frame. Frame times are relative to the start of the range.
/// Returns the number of frames written.
pub fn export_pictures<P: Pictures, F: Frames>(
    tl: &Timeline,
    range: Option<Range>,
    pixels: usize,
    src: &mut P,
    out: &mut F,
) -> Result<u64, ExportError<P::Error, F::Error>> {
    let r = range.unwrap_or(Range::new(0, tl.duration()));
    let r = Range::new(r.start.max(0), r.end.min(tl.duration()));
    if r.is_empty() || tl.fps_num == 0 || tl.fps_den == 0 {
        return Err(ExportError::Empty);
    }
    let mut buf: Vec<u32> = alloc::vec![0; pixels];
    // First frame at or after the range start.
    let per = tl.fps_den as i128 * SECOND as i128;
    let mut n = ((r.start as i128 * tl.fps_num as i128 + per - 1) / per) as u64;
    let mut written = 0;
    loop {
        let t = frame_time(tl, n);
        if t >= r.end {
            break;
        }
        match picture_at(tl, t) {
            Some((media, s)) => src.picture(media, s, &mut buf).map_err(ExportError::Picture)?,
            None => buf.fill(0),
        }
        out.frame(&buf, t - r.start).map_err(ExportError::Output)?;
        written += 1;
        n += 1;
    }
    Ok(written)
}

/// One piece of sound placed in the output, all in samples at the output rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundPiece {
    pub track: u32,
    pub media: u32,
    /// First output sample.
    pub at: u64,
    pub samples: u64,
    /// First source sample, at the output rate.
    pub source: u64,
    /// Per-mille speed and volume, as on the clip.
    pub speed: u32,
    pub volume: u32,
}

fn samples(t: Micros, rate: u32) -> u64 {
    (t.max(0) as i128 * rate as i128 / SECOND as i128) as u64
}

/// Every piece of sound the export mixes, from the sound tracks (muted tracks and
/// disabled clips left out). Edges are computed from absolute times, so consecutive
/// pieces meet without a gap or an overlap of even one sample.
pub fn sound_pieces(tl: &Timeline, rate: u32) -> Vec<SoundPiece> {
    let mut out = Vec::new();
    for tp in plan(tl).iter().filter(|p| p.kind == TrackKind::Audio) {
        for p in &tp.pieces {
            if let Piece::Play { media, timeline, source, speed, volume, .. } = p {
                let at = samples(timeline.start, rate);
                out.push(SoundPiece {
                    track: tp.track,
                    media: *media,
                    at,
                    samples: samples(timeline.end, rate) - at,
                    source: samples(source.start, rate),
                    speed: *speed,
                    volume: *volume,
                });
            }
        }
    }
    out
}

/// FNV-1a over frames, for gates that compare an export with its expected pictures.
#[derive(Clone, Copy, Debug)]
pub struct FrameHash(pub u64);

impl Default for FrameHash {
    fn default() -> Self {
        FrameHash(0xcbf2_9ce4_8422_2325)
    }
}

impl FrameHash {
    pub fn add(&mut self, pixels: &[u32]) {
        for p in pixels {
            for b in p.to_le_bytes() {
                self.0 ^= b as u64;
                self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
}
