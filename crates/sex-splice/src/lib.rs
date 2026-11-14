//! Splicy's editing core.
//!
//! Splicy is the SexOS video editor. This crate holds everything about an edit that is not
//! pixels: the timeline, the operations that change it, editing by transcript, suggestions
//! that wait for the editor, version history, the plan a renderer follows, and
//! OpenTimelineIO export. It has no dependencies and no I/O, so the same code runs in the
//! Splicy program, in a confined assistant, and in host tests.
//!
//! Ideas adapted from Scarlet (github.com/Forward-Future/scarlet, MIT): the original
//! footage is never touched, an edit is a list of operations on a cut list, cuts ripple
//! every track together so sound stays in sync, an assistant's edits can be staged as
//! suggestions, and a preview renders only the section an edit changed.
//!
//! Differences from Scarlet, by design: time is integer microseconds rather than floating
//! seconds, so a cut lands on the same instant on every run and goldens compare exactly;
//! speed is a rational per-mille value for the same reason.

#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod history;
pub mod otio;
pub mod render;
pub mod suggest;
pub mod transcript;

use alloc::string::String;
use alloc::vec::Vec;

/// Microseconds. Timeline and source times both use this unit.
pub type Micros = i64;

/// One second in [`Micros`].
pub const SECOND: Micros = 1_000_000;

/// Normal playback speed in per-mille.
pub const SPEED_NORMAL: u32 = 1000;

/// A half-open span of time, `start..end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Micros,
    pub end: Micros,
}

impl Range {
    pub const fn new(start: Micros, end: Micros) -> Self {
        Range { start, end }
    }
    pub fn len(&self) -> Micros {
        self.end - self.start
    }
    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
    pub fn overlaps(&self, other: &Range) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// What kind of picture or sound a piece of media holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Video,
    Audio,
    Image,
}

/// A source file. Splicy refers to it and never writes to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Media {
    pub id: u32,
    pub kind: MediaKind,
    /// Path inside the project's grant, as the file service names it.
    pub path: String,
    pub duration: Micros,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

/// A piece of a source placed on a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    pub id: u32,
    pub media: u32,
    /// Where the clip starts on the timeline.
    pub at: Micros,
    /// The part of the source it plays.
    pub source: Range,
    /// Per-mille playback speed; 1000 plays at normal speed.
    pub speed: u32,
    /// Linear gain in per-mille; 0 mutes.
    pub volume: u32,
    pub enabled: bool,
}

impl Clip {
    /// How long the clip lasts on the timeline.
    pub fn duration(&self) -> Micros {
        self.source.len() * SPEED_NORMAL as i64 / self.speed as i64
    }
    pub fn end(&self) -> Micros {
        self.at + self.duration()
    }
    pub fn span(&self) -> Range {
        Range::new(self.at, self.end())
    }
    /// The source time played at timeline time `t` (clamped to the clip).
    pub fn source_at(&self, t: Micros) -> Micros {
        let into = (t - self.at).clamp(0, self.duration());
        self.source.start + into * self.speed as i64 / SPEED_NORMAL as i64
    }
    /// The timeline time at which source time `s` plays, if the clip plays it.
    pub fn timeline_at(&self, s: Micros) -> Option<Micros> {
        if s < self.source.start || s > self.source.end {
            return None;
        }
        Some(self.at + (s - self.source.start) * SPEED_NORMAL as i64 / self.speed as i64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub id: u32,
    pub kind: TrackKind,
    pub name: String,
    /// Clips sorted by `at`, never overlapping.
    pub clips: Vec<Clip>,
    pub muted: bool,
}

/// A named point the editor or assistant left on the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub at: Micros,
    pub label: String,
}

/// The whole edit. Cloning is cheap enough for history at interview lengths.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Timeline {
    pub media: Vec<Media>,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
    /// Frames per second as a ratio, for snapping and export.
    pub fps_num: u32,
    pub fps_den: u32,
    next_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    NoTrack(u32),
    NoClip(u32),
    NoMedia(u32),
    /// The clip would cover part of another clip on the same track.
    Overlap { track: u32, clip: u32 },
    /// The source range lies outside the media, or is empty.
    BadSource,
    BadSpeed,
    /// A split point outside the clip.
    OutsideClip,
    /// The track kind does not fit the media (sound on a picture track).
    WrongTrack,
}

/// One change to a timeline. Every edit, by hand or by assistant, is a list of these, so
/// history, suggestions and replay share one vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    AddClip { track: u32, media: u32, at: Micros, source: Range },
    RemoveClip { clip: u32 },
    /// Cut one clip in two at timeline time `at`.
    Split { clip: u32, at: Micros },
    /// Change where a clip starts and stops playing its source.
    Trim { clip: u32, source: Range },
    Move { clip: u32, at: Micros },
    SetSpeed { clip: u32, speed: u32 },
    SetVolume { clip: u32, volume: u32 },
    SetEnabled { clip: u32, enabled: bool },
    /// Remove a span from every track and close the gap, keeping all tracks in sync.
    RippleDelete { range: Range },
    AddMarker { at: Micros, label: String },
}

impl Timeline {
    pub fn new(fps_num: u32, fps_den: u32) -> Self {
        Timeline { fps_num, fps_den, next_id: 1, ..Default::default() }
    }

    fn take_id(&mut self) -> u32 {
        let id = self.next_id.max(1);
        self.next_id = id + 1;
        id
    }

    pub fn add_media(&mut self, kind: MediaKind, path: &str, duration: Micros) -> u32 {
        let id = self.take_id();
        self.media.push(Media { id, kind, path: path.into(), duration });
        id
    }

    pub fn add_track(&mut self, kind: TrackKind, name: &str) -> u32 {
        let id = self.take_id();
        self.tracks.push(Track { id, kind, name: name.into(), clips: Vec::new(), muted: false });
        id
    }

    pub fn media(&self, id: u32) -> Option<&Media> {
        self.media.iter().find(|m| m.id == id)
    }

    pub fn clip(&self, id: u32) -> Option<(&Track, &Clip)> {
        self.tracks.iter().find_map(|t| t.clips.iter().find(|c| c.id == id).map(|c| (t, c)))
    }

    fn locate(&self, id: u32) -> Result<(usize, usize), EditError> {
        for (ti, t) in self.tracks.iter().enumerate() {
            if let Some(ci) = t.clips.iter().position(|c| c.id == id) {
                return Ok((ti, ci));
            }
        }
        Err(EditError::NoClip(id))
    }

    /// The end of the last clip on any track.
    pub fn duration(&self) -> Micros {
        self.tracks.iter().flat_map(|t| t.clips.iter()).map(Clip::end).max().unwrap_or(0)
    }

    /// The id the next added clip will get; suggestions use it to name clips in advance.
    pub fn peek_next_id(&self) -> u32 {
        self.next_id.max(1)
    }

    /// Apply a list of ops, all or nothing.
    pub fn apply_all(&self, ops: &[Op]) -> Result<Timeline, EditError> {
        let mut next = self.clone();
        for op in ops {
            next.apply(op)?;
        }
        Ok(next)
    }

    /// Apply one op in place. On error the timeline is unchanged.
    pub fn apply(&mut self, op: &Op) -> Result<(), EditError> {
        match op {
            Op::AddClip { track, media, at, source } => {
                let m = self.media(*media).ok_or(EditError::NoMedia(*media))?;
                let ti = self.tracks.iter().position(|t| t.id == *track).ok_or(EditError::NoTrack(*track))?;
                let fits = match (self.tracks[ti].kind, m.kind) {
                    (TrackKind::Audio, MediaKind::Image) => false,
                    _ => true,
                };
                if !fits {
                    return Err(EditError::WrongTrack);
                }
                if source.is_empty() || source.start < 0 || (m.kind != MediaKind::Image && source.end > m.duration) || *at < 0 {
                    return Err(EditError::BadSource);
                }
                let id = self.next_id.max(1);
                let clip = Clip { id, media: *media, at: *at, source: *source, speed: SPEED_NORMAL, volume: 1000, enabled: true };
                self.place(ti, clip)?;
                self.next_id = id + 1;
            }
            Op::RemoveClip { clip } => {
                let (ti, ci) = self.locate(*clip)?;
                self.tracks[ti].clips.remove(ci);
            }
            Op::Split { clip, at } => {
                let (ti, ci) = self.locate(*clip)?;
                let c = self.tracks[ti].clips[ci].clone();
                if *at <= c.at || *at >= c.end() {
                    return Err(EditError::OutsideClip);
                }
                let mid = c.source_at(*at);
                let id = self.take_id();
                let mut left = c.clone();
                left.source.end = mid;
                let mut right = c;
                right.id = id;
                right.source.start = mid;
                right.at = *at;
                // Rounding from speed can leave the left edge a microsecond short; the right
                // half keeps its exact timeline start so later cuts land where asked.
                let t = &mut self.tracks[ti];
                t.clips[ci] = left;
                t.clips.insert(ci + 1, right);
            }
            Op::Trim { clip, source } => {
                let (ti, ci) = self.locate(*clip)?;
                let mut c = self.tracks[ti].clips[ci].clone();
                let m = self.media(c.media).ok_or(EditError::NoMedia(c.media))?;
                if source.is_empty() || source.start < 0 || (m.kind != MediaKind::Image && source.end > m.duration) {
                    return Err(EditError::BadSource);
                }
                c.source = *source;
                self.replace(ti, ci, c)?;
            }
            Op::Move { clip, at } => {
                let (ti, ci) = self.locate(*clip)?;
                let mut c = self.tracks[ti].clips[ci].clone();
                if *at < 0 {
                    return Err(EditError::BadSource);
                }
                c.at = *at;
                self.replace(ti, ci, c)?;
            }
            Op::SetSpeed { clip, speed } => {
                if !(10..=100_000).contains(speed) {
                    return Err(EditError::BadSpeed);
                }
                let (ti, ci) = self.locate(*clip)?;
                let mut c = self.tracks[ti].clips[ci].clone();
                c.speed = *speed;
                self.replace(ti, ci, c)?;
            }
            Op::SetVolume { clip, volume } => {
                let (ti, ci) = self.locate(*clip)?;
                self.tracks[ti].clips[ci].volume = *volume;
            }
            Op::SetEnabled { clip, enabled } => {
                let (ti, ci) = self.locate(*clip)?;
                self.tracks[ti].clips[ci].enabled = *enabled;
            }
            Op::RippleDelete { range } => {
                if range.is_empty() {
                    return Ok(());
                }
                self.ripple_delete(*range);
            }
            Op::AddMarker { at, label } => {
                let i = self.markers.partition_point(|m| m.at <= *at);
                self.markers.insert(i, Marker { at: *at, label: label.clone() });
            }
        }
        Ok(())
    }

    fn replace(&mut self, ti: usize, ci: usize, clip: Clip) -> Result<(), EditError> {
        let old = self.tracks[ti].clips.remove(ci);
        if let Err(e) = self.place(ti, clip) {
            self.tracks[ti].clips.insert(ci, old);
            return Err(e);
        }
        Ok(())
    }

    fn place(&mut self, ti: usize, clip: Clip) -> Result<(), EditError> {
        let t = &mut self.tracks[ti];
        let span = clip.span();
        if let Some(other) = t.clips.iter().find(|c| c.span().overlaps(&span)) {
            return Err(EditError::Overlap { track: t.id, clip: other.id });
        }
        let i = t.clips.partition_point(|c| c.at < clip.at);
        t.clips.insert(i, clip);
        Ok(())
    }

    fn ripple_delete(&mut self, r: Range) {
        for t in self.tracks.iter_mut() {
            let mut out = Vec::with_capacity(t.clips.len() + 1);
            for c in t.clips.drain(..) {
                let span = c.span();
                if span.end <= r.start {
                    out.push(c);
                } else if span.start >= r.end {
                    let mut c = c;
                    c.at -= r.len();
                    out.push(c);
                } else {
                    // The clip meets the cut: keep what plays before it and what plays after.
                    if span.start < r.start {
                        let mut left = c.clone();
                        left.source.end = c.source_at(r.start);
                        out.push(left);
                    }
                    if span.end > r.end {
                        let mut right = c.clone();
                        right.source.start = c.source_at(r.end);
                        right.at = r.start;
                        if span.start < r.start {
                            right.id = 0; // both halves survive: the right one gets a fresh id below
                        }
                        out.push(right);
                    }
                }
            }
            t.clips = out;
        }
        for t in self.tracks.iter_mut() {
            for c in t.clips.iter_mut().filter(|c| c.id == 0) {
                let id = self.next_id.max(1);
                self.next_id = id + 1;
                c.id = id;
            }
        }
        let len = r.len();
        self.markers.retain(|m| !(m.at >= r.start && m.at < r.end));
        for m in self.markers.iter_mut().filter(|m| m.at >= r.end) {
            m.at -= len;
        }
    }

    /// Where on the timeline source time `s` of `media` plays, on the first track that
    /// plays it. Transcript words use this to find themselves after earlier cuts.
    pub fn place_of(&self, media: u32, s: Micros) -> Option<Micros> {
        self.tracks.iter().flat_map(|t| t.clips.iter()).filter(|c| c.media == media && c.enabled).find_map(|c| {
            if s >= c.source.start && s < c.source.end {
                c.timeline_at(s)
            } else {
                None
            }
        })
    }
}

#[cfg(test)]
mod tests;
