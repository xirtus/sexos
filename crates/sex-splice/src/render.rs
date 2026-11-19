//! What a renderer reads: for each track, the pieces of source to play in order, with gaps.
//! The decoder and encoder (sex-media's FFmpeg) follow this plan; this crate never touches
//! pixels or samples.
//!
//! [`changed_window`] finds the part of the timeline an edit changed, so a preview renders
//! only that section with a little context on each side, as Scarlet does.

use crate::{Micros, Range, Timeline, TrackKind};
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    /// Black picture or silence.
    Gap { timeline: Range },
    Play { clip: u32, media: u32, timeline: Range, source: Range, speed: u32, volume: u32 },
}

impl Piece {
    pub fn timeline(&self) -> Range {
        match self {
            Piece::Gap { timeline } | Piece::Play { timeline, .. } => *timeline,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackPlan {
    pub track: u32,
    pub kind: TrackKind,
    pub pieces: Vec<Piece>,
}

/// The render plan for the whole timeline. Disabled clips and muted audio tracks play as gaps.
pub fn plan(tl: &Timeline) -> Vec<TrackPlan> {
    let end = tl.duration();
    tl.tracks
        .iter()
        .map(|t| {
            let mut pieces = Vec::new();
            let mut at = 0;
            for c in &t.clips {
                let silent = !c.enabled || (t.muted && t.kind == TrackKind::Audio);
                if silent {
                    continue;
                }
                if c.at > at {
                    pieces.push(Piece::Gap { timeline: Range::new(at, c.at) });
                }
                pieces.push(Piece::Play { clip: c.id, media: c.media, timeline: c.span(), source: c.source, speed: c.speed, volume: c.volume });
                at = c.end();
            }
            if end > at {
                pieces.push(Piece::Gap { timeline: Range::new(at, end) });
            }
            merge_gaps(&mut pieces);
            TrackPlan { track: t.id, kind: t.kind, pieces }
        })
        .collect()
}

fn merge_gaps(p: &mut Vec<Piece>) {
    let mut out: Vec<Piece> = Vec::with_capacity(p.len());
    for piece in p.drain(..) {
        if let (Some(Piece::Gap { timeline: last }), Piece::Gap { timeline }) = (out.last_mut(), &piece) {
            last.end = timeline.end;
            continue;
        }
        out.push(piece);
    }
    *p = out;
}

/// A piece as a function of time: at timeline `t` in `start..end` it plays source
/// `src + dir * (t - start) * speed / 1000` of `media`, or nothing for a gap.
#[derive(Clone, Copy)]
struct Atom {
    start: Micros,
    end: Micros,
    play: Option<(u32, Micros, u32, u32)>,
    dir: i64,
}

impl Atom {
    fn agrees(&self, o: &Atom, t: Micros) -> bool {
        match (self.play, o.play) {
            (None, None) => true,
            (Some((m, s, sp, v)), Some((m2, s2, sp2, v2))) => {
                m == m2 && sp == sp2 && v == v2 && self.dir == o.dir && {
                    let a = s + self.dir * (t - self.start) * sp as i64 / 1000;
                    let b = s2 + o.dir * (t - o.start) * sp2 as i64 / 1000;
                    a == b
                }
            }
            _ => false,
        }
    }
}

/// The atoms of one track, front to back, or (with `reverse`) measured from the end.
fn atoms(plan: Option<&TrackPlan>, duration: Micros, reverse: bool) -> Vec<Atom> {
    let mut v: Vec<Atom> = plan
        .map(|t| {
            t.pieces
                .iter()
                .map(|p| {
                    let r = p.timeline();
                    let play = |src: Micros| match p {
                        Piece::Gap { .. } => None,
                        Piece::Play { media, speed, volume, .. } => Some((*media, src, *speed, *volume)),
                    };
                    let (src_start, src_end) = match p {
                        Piece::Play { source, .. } => (source.start, source.end),
                        Piece::Gap { .. } => (0, 0),
                    };
                    if reverse {
                        Atom { start: duration - r.end, end: duration - r.start, play: play(src_end), dir: -1 }
                    } else {
                        Atom { start: r.start, end: r.end, play: play(src_start), dir: 1 }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if reverse {
        v.reverse();
    }
    v
}

/// How far from the start two atom lists play the same thing.
fn agree_until(a: &[Atom], b: &[Atom]) -> Micros {
    let (mut i, mut j) = (0, 0);
    let mut t = 0;
    while i < a.len() && j < b.len() {
        if !a[i].agrees(&b[j], t) {
            return t;
        }
        let next = a[i].end.min(b[j].end);
        // Check the last instant of the overlap too: a speed change agrees only at its start.
        if next > t && !a[i].agrees(&b[j], next - 1) {
            return t;
        }
        t = next;
        if a[i].end == t {
            i += 1;
        }
        if b[j].end == t {
            j += 1;
        }
    }
    if i == a.len() && j == b.len() {
        Micros::MAX
    } else {
        t
    }
}

/// The span of `after` that differs from `before`, widened by `context` on each side and
/// clamped to the timeline. `None` when the edit changed nothing a viewer would notice.
pub fn changed_window(before: &Timeline, after: &Timeline, context: Micros) -> Option<Range> {
    let (b, a) = (plan(before), plan(after));
    let (bd, ad) = (before.duration(), after.duration());
    let mut ids: Vec<u32> = before.tracks.iter().chain(after.tracks.iter()).map(|t| t.id).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut front = Micros::MAX;
    let mut back = Micros::MAX;
    for id in ids {
        let (pb, pa) = (b.iter().find(|p| p.track == id), a.iter().find(|p| p.track == id));
        let f = agree_until(&atoms(pb, bd, false), &atoms(pa, ad, false));
        if f == Micros::MAX {
            continue;
        }
        front = front.min(f);
        back = back.min(agree_until(&atoms(pb, bd, true), &atoms(pa, ad, true)));
    }
    if front == Micros::MAX {
        return None;
    }
    let end = (ad - back.min(ad)).max(front.min(ad));
    let start = front.min(ad);
    Some(Range::new((start - context).max(0), (end + context).min(ad)))
}
